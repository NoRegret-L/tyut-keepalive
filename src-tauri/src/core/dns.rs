use anyhow::{bail, Context, Result};
use std::{
    net::{Ipv4Addr, UdpSocket},
    time::Duration,
};

use crate::{log, LOGIN_DOMAIN};

const CAMPUS_DNS: [&str; 2] = ["202.207.240.225", "202.207.247.227"];

fn get_u16(data: &[u8], offset: usize) -> Result<u16> {
    let bytes = data.get(offset..offset + 2).context("DNS 数据被截断")?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn skip_dns_name(data: &[u8], mut offset: usize) -> Result<usize> {
    loop {
        let len = *data.get(offset).context("DNS 名称被截断")?;
        if len == 0 {
            return Ok(offset + 1);
        }
        if len & 0xc0 == 0xc0 {
            data.get(offset + 1).context("DNS 指针被截断")?;
            return Ok(offset + 2);
        }
        if len & 0xc0 != 0 {
            bail!("DNS 标签长度非法");
        }
        offset += 1 + len as usize;
        if offset > data.len() {
            bail!("DNS 标签超出数据范围");
        }
    }
}

fn parse_dns_response(data: &[u8], expected_id: u16) -> Result<Vec<Ipv4Addr>> {
    if data.len() < 12 || get_u16(data, 0)? != expected_id {
        bail!("DNS 响应 ID 不匹配或长度不足");
    }
    let flags = get_u16(data, 2)?;
    if flags & 0x8000 == 0 || flags & 0x000f != 0 || flags & 0x0200 != 0 {
        bail!("DNS 响应标志错误或被截断");
    }
    let mut offset = 12;
    for _ in 0..get_u16(data, 4)? {
        offset = skip_dns_name(data, offset)?;
        data.get(offset..offset + 4).context("DNS 问题不完整")?;
        offset += 4;
    }
    let mut addresses = Vec::new();
    for _ in 0..get_u16(data, 6)? {
        offset = skip_dns_name(data, offset)?;
        data.get(offset..offset + 10).context("DNS 记录头不完整")?;
        let record_type = get_u16(data, offset)?;
        let record_class = get_u16(data, offset + 2)?;
        let data_length = get_u16(data, offset + 8)? as usize;
        offset += 10;
        let raw = data
            .get(offset..offset + data_length)
            .context("DNS 记录值不完整")?;
        if record_type == 1 && record_class == 1 && data_length == 4 {
            let addr = Ipv4Addr::new(raw[0], raw[1], raw[2], raw[3]);
            if !addresses.contains(&addr) {
                addresses.push(addr);
            }
        }
        offset += data_length;
    }
    if addresses.is_empty() {
        bail!("校园 DNS 没有返回 IPv4 A 记录");
    }
    Ok(addresses)
}

fn query_campus_dns(server: &str, host: &str) -> Result<Vec<Ipv4Addr>> {
    let id: u16 = rand::random();
    let mut packet = Vec::new();
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&0x0100_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&[0_u8; 6]);
    for part in host.split('.') {
        if part.is_empty() || part.len() > 63 || !part.is_ascii() {
            bail!("DNS 查询域名无效");
        }
        packet.push(part.len() as u8);
        packet.extend_from_slice(part.as_bytes());
    }
    packet.extend_from_slice(&[0, 0, 1, 0, 1]);
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    socket.connect(format!("{server}:53"))?;
    socket.send(&packet)?;
    let mut buf = [0_u8; 4096];
    let len = socket.recv(&mut buf)?;
    parse_dns_response(&buf[..len], id)
}

pub(crate) fn campus_host_ips() -> Result<Vec<Ipv4Addr>> {
    let mut errors = Vec::new();
    for server in CAMPUS_DNS {
        match query_campus_dns(server, LOGIN_DOMAIN) {
            Ok(addresses) => {
                log("校园 DNS 解析成功");
                return Ok(addresses);
            }
            Err(error) => errors.push(format!("{server}: {error:#}")),
        }
    }
    bail!("校园 DNS 解析失败：{}", errors.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dns_a_record() {
        let mut dns = vec![0x12, 0x34, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
        dns.extend_from_slice(&[3, b'f', b'o', b'o', 0, 0, 1, 0, 1]);
        dns.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 30, 0, 4, 1, 2, 3, 4]);
        assert_eq!(
            parse_dns_response(&dns, 0x1234).unwrap(),
            vec![Ipv4Addr::new(1, 2, 3, 4)]
        );
    }

    #[test]
    fn reject_wrong_dns_id() {
        let dns = [0u8; 12];
        assert!(parse_dns_response(&dns, 0x1234).is_err());
    }
}
