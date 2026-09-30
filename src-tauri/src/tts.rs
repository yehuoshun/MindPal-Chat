use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio_tungstenite::tungstenite::Message;

/// 常量与算法来源：edge-tts (Python) 6.x 源码，2026-09 实测有效
const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const WSS_URL: &str = "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";
const SEC_MS_GEC_VERSION: &str = "1-143.0.3650.75";
const WIN_EPOCH: f64 = 11_644_473_600.0;
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0";

// ---------- Sec-MS-GEC（DRM token，算法锁定自 edge-tts drm.py） ----------

/// unix 秒 → Sec-MS-GEC：加 Windows 纪元，向下取整到 5 分钟，转 100ns ticks，
/// 拼 TrustedClientToken 后 SHA256，输出大写 hex
pub fn sec_ms_gec(unix_seconds: f64) -> String {
    let mut ticks = unix_seconds + WIN_EPOCH;
    ticks -= ticks % 300.0;
    ticks *= 10_000_000.0;
    let to_hash = format!("{:.0}{}", ticks, TRUSTED_CLIENT_TOKEN);
    let mut h = Sha256::new();
    h.update(to_hash.as_bytes());
    let digest = h.finalize();
    digest.iter().map(|b| format!("{b:02X}")).collect()
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn uuid_hex() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

// ---------- GMT 时间串（JS Date().toString() 风格，服务器要求） ----------

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// Howard Hinnant civil_from_days：天数(自1970) → (年, 月, 日)
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

pub fn gmt_date_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    let (y, mo, d) = civil_from_days(days);
    let wd = WEEKDAYS[(days + 4).rem_euclid(7) as usize];
    format!(
        "{} {} {:02} {} {:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
        wd,
        MONTHS[(mo - 1) as usize],
        d,
        y,
        hh,
        mm,
        ss
    )
}

// ---------- 消息构建 ----------

/// SSML 文本转义（& 必须最先处理）
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn speech_config() -> String {
    format!(
        "X-Timestamp:{}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n\
{{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}\r\n",
        gmt_date_string()
    )
}

pub fn build_ssml(voice: &str, text: &str) -> String {
    format!(
        "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}Z\r\nPath:ssml\r\n\r\n\
<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
<voice name='{}'><prosody pitch='+0Hz' rate='+0%' volume='+0%'>{}</prosody></voice></speak>",
        uuid_hex(),
        gmt_date_string(),
        voice,
        xml_escape(text)
    )
}

// ---------- 响应解析 ----------

/// 从文本消息头部取 Path 值
pub fn parse_path(text: &str) -> Option<&str> {
    let headers = text.split("\r\n\r\n").next()?;
    for line in headers.lines() {
        if let Some(v) = line.strip_prefix("Path:") {
            return Some(v.trim());
        }
    }
    None
}

/// 解析音频二进制块：前 2 字节 = header 长度；header 含 `Path: audio` 时返回音频数据
pub fn parse_audio_chunk(b: &[u8]) -> Option<&[u8]> {
    if b.len() < 2 {
        return None;
    }
    let header_len = u16::from_be_bytes([b[0], b[1]]) as usize;
    if 2 + header_len > b.len() {
        return None;
    }
    let header = std::str::from_utf8(&b[2..2 + header_len]).ok()?;
    if header.lines().any(|l| l.starts_with("Path:") && l[5..].trim() == "audio") {
        Some(&b[2 + header_len..])
    } else {
        None
    }
}

// ---------- 合成 ----------

/// 文本 → mp3 写入 out_path。连接 → speech.config → ssml → 收音频 → turn.end 结束
pub async fn synthesize(text: &str, voice: &str, out_path: &Path) -> Result<(), String> {
    let url = format!(
        "{WSS_URL}?TrustedClientToken={TRUSTED_CLIENT_TOKEN}&ConnectionId={}&Sec-MS-GEC={}&Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}",
        uuid_hex(),
        sec_ms_gec(unix_now()),
    );
    let req = http::Request::builder()
        .uri(&url)
        .header("Origin", "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold")
        .header("User-Agent", UA)
        .header("Pragma", "no-cache")
        .header("Cache-Control", "no-cache")
        .header("Sec-WebSocket-Version", "13")
        .header("Cookie", format!("muid={};", uuid_hex().to_uppercase()))
        .body(())
        .map_err(|e| format!("构建请求失败: {e}"))?;

    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|e| format!("连接 Edge TTS 失败: {e}"))?;

    ws.send(Message::Text(speech_config().into()))
        .await
        .map_err(|e| format!("发送 speech.config 失败: {e}"))?;
    ws.send(Message::Text(build_ssml(voice, text).into()))
        .await
        .map_err(|e| format!("发送 SSML 失败: {e}"))?;

    let mut audio: Vec<u8> = Vec::new();
    while let Some(msg) = ws.next().await {
        let msg = msg.map_err(|e| format!("接收失败: {e}"))?;
        match msg {
            Message::Text(t) => {
                if parse_path(&t) == Some("turn.end") {
                    break;
                }
            }
            Message::Binary(b) => {
                if let Some(data) = parse_audio_chunk(&b) {
                    audio.extend_from_slice(data);
                }
            }
            _ => {}
        }
    }

    if audio.is_empty() {
        return Err("TTS 未收到音频数据（可能语音名无效或服务限流）".to_string());
    }
    std::fs::write(out_path, &audio).map_err(|e| format!("写入音频失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_ms_gec_matches_python_reference() {
        // 期望值由 edge-tts drm.generate_sec_ms_gec 同算法 + 固定时间戳算出
        assert_eq!(
            sec_ms_gec(1_770_000_000.0),
            "FF7809BFC19BE2038B685B2EA98A7C63607BFD3FBF97B1712584D6A4690DAC54"
        );
        // 同一时刻幂等
        assert_eq!(sec_ms_gec(1_770_000_000.0), sec_ms_gec(1_770_000_000.0));
    }

    #[test]
    fn civil_from_days_reference() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_698), (2023, 12, 7));
        assert_eq!(civil_from_days(20_587), (2026, 5, 14));
        assert_eq!(civil_from_days(7_387), (1990, 3, 24));
    }

    #[test]
    fn gmt_string_format() {
        let s = gmt_date_string();
        assert!(s.ends_with("GMT+0000 (Coordinated Universal Time)"));
        // 星期几取决于 UTC 时刻，不硬编码；只校验是合法星期缩写 + 月份缩写
        assert!(WEEKDAYS.iter().any(|w| s.starts_with(w)));
        assert!(MONTHS.iter().any(|m| s.contains(m)));
        assert!(s.contains("GMT+0000 (Coordinated Universal Time)"));
    }

    #[test]
    fn xml_escape_order() {
        assert_eq!(xml_escape("a&b<c>d\"e"), "a&amp;b&lt;c&gt;d&quot;e");
        assert_eq!(xml_escape("&amp;"), "&amp;amp;");
    }

    #[test]
    fn ssml_contains_voice_and_escaped_text() {
        let ssml = build_ssml("zh-CN-XiaoxiaoNeural", "你好 & 再见");
        assert!(ssml.contains("<voice name='zh-CN-XiaoxiaoNeural'>"));
        assert!(ssml.contains("你好 &amp; 再见"));
        assert!(ssml.starts_with("X-RequestId:"));
        assert!(ssml.contains("\r\nPath:ssml\r\n\r\n"));
    }

    #[test]
    fn parse_path_extracts_turn_end() {
        assert_eq!(parse_path("X-RequestId:abc\r\nPath:turn.end\r\n\r\n"), Some("turn.end"));
        assert_eq!(parse_path("Path:response\r\n\r\n{}"), Some("response"));
        assert_eq!(parse_path("no headers here"), None);
    }

    #[test]
    fn parse_audio_chunk_extracts_data() {
        let header = "Path: audio\r\nContent-Type: audio/mpeg\r\n\r\n";
        let mut chunk = vec![0u8, header.len() as u8];
        chunk.extend_from_slice(header.as_bytes());
        chunk.extend_from_slice(&[0x49, 0x44, 0x33]);
        let data = parse_audio_chunk(&chunk).unwrap();
        assert_eq!(data, &[0x49, 0x44, 0x33]);
        // 非 audio path 返回 None
        let bad = b"\x00\x05Path: xxx\r\n\r\nabc".to_vec();
        assert!(parse_audio_chunk(&bad).is_none());
    }
}
