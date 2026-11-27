use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

const XOR_KEY: u32 = 0x77;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Credentials {
    pub(crate) account: String,
    pub(crate) password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) saved_at: Option<String>,
}

// [设计意图/根因] -> 保持与原 ePortal 协议及历史 Python 版逐字节兼容
// [理论先验/数学假设] -> 对每个字符码点异或固定密钥 0x77，十六进制定长两位
// [机制闭环解释] -> 编码值可直接作为登录参数发送；这是协议混淆而非加密，不得作为隐私保护
pub(crate) fn xor_encode(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{:02x}", (ch as u32) ^ XOR_KEY);
    }
    out
}

pub(crate) fn xor_decode(encoded: &str) -> Option<String> {
    let mut out = String::new();
    let bytes: Vec<char> = encoded.chars().collect();
    if bytes.len() % 2 != 0 {
        return None;
    }
    for pair in bytes.chunks(2) {
        let hex: String = pair.iter().collect();
        let byte = u32::from_str_radix(&hex, 16).ok()?;
        out.push(char::from_u32(byte ^ XOR_KEY)?);
    }
    Some(out)
}

pub(crate) fn masked_account(encoded: &str) -> String {
    let Some(account) = xor_decode(encoded) else {
        return "未知".to_string();
    };
    let chars: Vec<char> = account.chars().collect();
    if chars.len() <= 4 {
        return "****".to_string();
    }
    let suffix: String = chars[chars.len() - 2..].iter().collect();
    format!("{}****{}", chars[0], suffix)
}

pub(crate) fn validate(credentials: &Credentials) -> Result<()> {
    // Preserve the exact historical protocol encoding; do not decode and re-encode
    // non-ASCII credentials, whose legacy hex representation can be ambiguous.
    for value in [&credentials.account, &credentials.password] {
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("凭据格式无效");
        }
    }
    Ok(())
}

pub(crate) fn read_credentials(path: &Path) -> Result<Option<Credentials>> {
    let data = match fs::read_to_string(path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("读取旧凭据文件失败"),
    };
    let credentials: Credentials = serde_json::from_str(&data)
        .map_err(|_| anyhow::anyhow!("config.enc 格式有误"))?;
    validate(&credentials)?;
    Ok(Some(credentials))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_matches_python_protocol() {
        assert_eq!(xor_encode("dr1003"), "130546474744");
        assert_eq!(xor_encode("1"), "46");
        assert_eq!(xor_encode("中"), format!("{:02x}", ('中' as u32) ^ 0x77));
    }

    #[test]
    fn xor_round_trip_and_mask() {
        let encoded = xor_encode("dr1003");
        assert_eq!(xor_decode(&encoded).as_deref(), Some("dr1003"));
        assert_eq!(masked_account(&encoded), "d****03");
    }
}
