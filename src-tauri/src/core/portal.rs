use anyhow::{bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use regex::Regex;
use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue, ACCEPT, LOCATION, REFERER, USER_AGENT},
    redirect::Policy,
    Url,
};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    config::{xor_encode, Credentials},
    dns::campus_host_ips,
    log, LOGIN_DOMAIN, LOGIN_HOST, LOGIN_USER_AGENT, PORTAL_URL, REFERER_URL, TARGET,
    USER_AGENT_VALUE,
};

fn local_ip() -> Result<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect(format!("{TARGET}:80"))?;
    match socket.local_addr()?.ip() {
        IpAddr::V4(ip) => Ok(ip),
        _ => bail!("无法获取 IPv4 本地地址"),
    }
}

fn local_mac(ip: Ipv4Addr) -> Result<String> {
    for interface in netdev::get_interfaces() {
        if interface.ipv4_addrs().contains(&ip) {
            if let Some(mac) = interface.mac_addr {
                return Ok(mac.to_string().replace(':', "").to_ascii_lowercase());
            }
        }
    }
    bail!("未找到 {ip} 对应网卡的 MAC；可通过 --mac 手动指定")
}

pub(crate) fn client(campus_ips: Option<Vec<Ipv4Addr>>) -> Result<Client> {
    let mut builder = Client::builder()
        .no_proxy()
        .cookie_store(true)
        .redirect(Policy::none())
        .timeout(Duration::from_secs(15));
    if let Some(ips) = campus_ips {
        let addrs: Vec<_> = ips
            .into_iter()
            .map(|ip| SocketAddr::new(IpAddr::V4(ip), 804))
            .collect();
        builder = builder.resolve_to_addrs(LOGIN_DOMAIN, &addrs);
    }
    Ok(builder.build()?)
}

pub(crate) fn check_network(client: &Client) -> bool {
    if let Ok(response) = client
        .get(format!("http://{TARGET}"))
        .timeout(Duration::from_secs(3))
        .send()
    {
        if response.status().as_u16() == 200 {
            return true;
        }
    }
    if let Ok(response) = client
        .get(PORTAL_URL)
        .timeout(Duration::from_secs(5))
        .send()
    {
        if matches!(response.status().as_u16(), 301 | 302) {
            return true;
        }
        if response.status().is_success() {
            if let Ok(body) = response.text() {
                return !body.contains("v46ip") && !body.contains("wlan_user_ip");
            }
        }
    }
    false
}

fn portal_ip(client: &Client, cancelled: &AtomicBool) -> Result<Option<Ipv4Addr>> {
    anyhow::ensure!(!cancelled.load(Ordering::Acquire), "认证任务已取消");
    log(format!("访问 Portal：{PORTAL_URL}"));
    let mut url = Url::parse(PORTAL_URL)?;
    let mut response = client
        .get(url.clone())
        .timeout(Duration::from_secs(10))
        .send()?;
    for _ in 0..10 {
        anyhow::ensure!(!cancelled.load(Ordering::Acquire), "认证任务已取消");
        if !response.status().is_redirection() {
            break;
        }
        let location = match response.headers().get(LOCATION) {
            Some(header) => header.to_str()?.to_owned(),
            None => break,
        };
        url = url.join(&location)?;
        // Preserve campus redirect compatibility, but reject non-HTTP schemes
        // and embedded user credentials; authentication always uses LOGIN_HOST.
        anyhow::ensure!(matches!(url.scheme(), "http" | "https") && url.username().is_empty() && url.password().is_none(),
            "Portal 重定向地址不安全");
        log("Portal 收到重定向响应");
        response = client
            .get(url.clone())
            .timeout(Duration::from_secs(10))
            .send()?;
    }
    if response.status().is_redirection() && response.headers().contains_key(LOCATION) {
        bail!("Portal 重定向次数过多");
    }
    if !response.status().is_success() {
        bail!("Portal HTTP 状态异常：{}", response.status());
    }
    let html = response.text()?;
    for pattern in [
        r#"v46ip\s*[:=]\s*["']([^"']+)"#,
        r#"wlan_user_ip["']?\s*[:=]\s*["']([^"']+)"#,
    ] {
        let re = Regex::new(pattern)?;
        if let Some(ip) = re
            .captures(&html)
            .and_then(|capture| capture.get(1))
            .and_then(|matched| matched.as_str().parse::<Ipv4Addr>().ok())
        {
            log("Portal 提取到客户端 IP");
            return Ok(Some(ip));
        }
    }
    Ok(None)
}

fn protocol_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(REFERER, HeaderValue::from_static(REFERER_URL));
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    headers
}

fn timestamp_param() -> Result<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    Ok(((now % 10_000) + 500).to_string())
}

fn load_config(client: &Client, ip: Ipv4Addr, mac: &str) -> Result<()> {
    let ip_encoded = xor_encode(&STANDARD.encode(ip.to_string()));
    let mac_encoded = xor_encode(mac);
    let params = vec![
        ("callback", xor_encode("dr1003")),
        ("program_index", String::new()),
        ("wlan_vlan_id", xor_encode("46")),
        ("wlan_user_ip", ip_encoded),
        ("wlan_user_ipv6", String::new()),
        ("wlan_user_ssid", String::new()),
        ("wlan_user_areaid", String::new()),
        ("wlan_ac_ip", String::new()),
        ("wlan_ap_mac", mac_encoded.clone()),
        ("gw_id", mac_encoded),
        ("jsVersion", xor_encode("4.X")),
        ("encrypt", "1".to_owned()),
        ("v", timestamp_param()?),
        ("lang", "zh".to_owned()),
    ];
    log("请求 loadConfig ...");
    let response = client
        .get(format!("{LOGIN_HOST}/eportal/portal/page/loadConfig"))
        .headers(protocol_headers())
        .query(&params)
        .timeout(Duration::from_secs(15))
        .send()
        .map_err(|error| error.without_url())?;
    let status = response.status();
    if !status.is_success() {
        bail!("loadConfig 返回 HTTP {status}");
    }
    let _body = response.text().map_err(|error| error.without_url())?;
    log("loadConfig 请求完成");
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoginOutcome {
    Success,
    BadPassword,
    Indeterminate,
    Reachable,
}

pub(crate) fn interpret_login(text: &str) -> LoginOutcome {
    let compact: String = text
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    if (text.contains("认证成功") && compact.contains("\"result\":1")) || text.contains("已在线")
    {
        LoginOutcome::Success
    } else if text.contains("密码错误") {
        LoginOutcome::BadPassword
    } else {
        LoginOutcome::Indeterminate
    }
}

fn login(client: &Client, ip: Ipv4Addr, credentials: &Credentials, cancelled: &AtomicBool) -> Result<LoginOutcome> {
    let params = vec![
        ("callback", xor_encode("dr1004")),
        ("login_method", xor_encode("1")),
        ("user_account", credentials.account.clone()),
        ("user_password", credentials.password.clone()),
        ("wlan_user_ip", xor_encode(&ip.to_string())),
        ("wlan_user_ipv6", String::new()),
        ("wlan_user_mac", xor_encode("000000000000")),
        ("wlan_ac_ip", String::new()),
        ("wlan_ac_name", String::new()),
        ("mac_type", xor_encode("0")),
        ("authex_enable", String::new()),
        ("jsVersion", xor_encode("4.3")),
        ("web", xor_encode("0")),
        ("terminal_type", xor_encode("1")),
        ("lang", xor_encode("zh-cn")),
        ("user_agent", xor_encode(LOGIN_USER_AGENT)),
        ("enable_r3", xor_encode("0")),
        ("v", timestamp_param()?),
        ("encrypt", "1".to_owned()),
    ];
    anyhow::ensure!(!cancelled.load(Ordering::Acquire), "认证任务已取消");
    log("请求 login ...");
    let response = client
        .get(format!("{LOGIN_HOST}/eportal/portal/login"))
        .headers(protocol_headers())
        .query(&params)
        .timeout(Duration::from_secs(15))
        .send()
        .map_err(|error| error.without_url())?;
    let status = response.status();
    let body = response.text().map_err(|error| error.without_url())?;
    if !status.is_success() {
        bail!("登录接口 HTTP 状态异常：{status}");
    }
    let outcome = interpret_login(&body);
    match outcome {
        LoginOutcome::Success => log("登录接口确认成功或已在线"),
        LoginOutcome::BadPassword => log("登录失败：密码错误"),
        LoginOutcome::Indeterminate => log("登录响应无法确认结果；将检测网络连通性"),
        LoginOutcome::Reachable => (),
    }
    Ok(outcome)
}

pub(crate) fn attempt_login(credentials: &Credentials, mac_override: Option<&str>, cancelled: &AtomicBool) -> Result<LoginOutcome> {
    anyhow::ensure!(!cancelled.load(Ordering::Acquire), "认证任务已取消");
    let ip = local_ip()?;
    let mac = match mac_override {
        Some(mac) => mac.to_owned(),
        None => local_mac(ip)?,
    };
    log("已获取本机网络接口信息");
    let resolved_ips = match campus_host_ips() {
        Ok(ips) => Some(ips),
        Err(_) => {
            log("校园 DNS 不可用，改用系统 DNS");
            None
        }
    };
    anyhow::ensure!(!cancelled.load(Ordering::Acquire), "认证任务已取消");
    let client = client(resolved_ips)?;
    let user_ip = portal_ip(&client, cancelled)?.unwrap_or(ip);
    log("已确定认证使用的客户端 IP");
    anyhow::ensure!(!cancelled.load(Ordering::Acquire), "认证任务已取消");
    load_config(&client, user_ip, &mac)?;
    thread::sleep(Duration::from_millis(500));
    let outcome = login(&client, user_ip, credentials, cancelled)?;
    anyhow::ensure!(!cancelled.load(Ordering::Acquire), "认证任务已取消");
    match outcome {
        LoginOutcome::Indeterminate if check_network(&client) => Ok(LoginOutcome::Reachable),
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_status_is_conservative_for_timeout() {
        assert_eq!(
            interpret_login(r#"dr1004({"result":1,"msg":"认证成功"})"#),
            LoginOutcome::Success
        );
        assert_eq!(interpret_login("已在线"), LoginOutcome::Success);
        assert_eq!(interpret_login("密码错误"), LoginOutcome::BadPassword);
        assert_eq!(interpret_login("认证超时"), LoginOutcome::Indeterminate);
    }
}
