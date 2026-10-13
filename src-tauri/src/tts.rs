use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use std::path::Path;
use tokio_tungstenite::tungstenite::Message;

/// 常量与算法来源：edge-tts (Python) 6.x 源码，2026-09 实测有效；
/// 2026-10 微软废弃 Sec-MS-GEC v1（旧算法返回 403），升级 v2（见 edge-tts PR #492）
const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const WSS_URL: &str = "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";
const SEC_MS_GEC_VERSION: &str = "2-143.0.3650.75";
const WIN_EPOCH: f64 = 11_644_473_600.0;
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0";

// ---------- Sec-MS-GEC v2（DRM token，算法锁定自 edge-tts drm.py，PR #492） ----------

// FIPS 180-4 常量（edge-tts 定制版，非标准 SHA-256 序列）
const SHA256_INITIAL: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];
const SHA256_STATE: [u32; 8] = [
    0xFD9FBEA2, 0x91243CE7, 0xBE499094, 0xCD2E1E0A, 0x6C006F49, 0x04BD35AB, 0xFEB0A7E4, 0x03EFFBD2,
];
const SHA256_SCHEDULE: [u32; 14] = [
    0xEB24BF9E, 0x1558691F, 0xA0C09EE2, 0x0CFA32BA, 0xA622EE8C, 0xD1DD9491, 0xC0DD25A0, 0xF9E70AB6,
    0xF0C31573, 0x7058CE4E, 0x7188E606, 0x00E5AB33, 0xD258B060, 0x72766564,
];
const SHA256_ROUND_CONSTANTS: [u32; 64] = [
    0x428A2F98, 0x71374491, 0xB5C0FBCF, 0xE9B5DBA5, 0x3956C25B, 0x59F111F1, 0x923F82A4, 0xAB1C5ED5,
    0xD807AA98, 0x12835B01, 0x243185BE, 0x550C7DC3, 0x72BE5D74, 0x80DEB1FE, 0x9BDC06A7, 0xC19BF174,
    0xE49B69C1, 0xEFBE4786, 0x0FC19DC6, 0x240CA1CC, 0x2DE92C6F, 0x4A7484AA, 0x5CB0A9DC, 0x76F988DA,
    0x983E5152, 0xA831C66D, 0xB00327C8, 0xBF597FC7, 0xC6E00BF3, 0xD5A79147, 0x06CA6351, 0x14292967,
    0x27B70A85, 0x2E1B2138, 0x4D2C6DFC, 0x53380D13, 0x650A7354, 0x766A0ABB, 0x81C2C92E, 0x92722C85,
    0xA2BFE8A1, 0xA81A664B, 0xC24B8B70, 0xC76C51A3, 0xD192E819, 0xD6990624, 0xF40E3585, 0x106AA070,
    0x19A4C116, 0x1E376C08, 0x2748774C, 0x34B0BCB5, 0x391C0CB3, 0x4ED8AA4A, 0x5B9CCA4F, 0x682E6FF3,
    0x748F82EE, 0x78A5636F, 0x84C87814, 0x8CC70208, 0x90BEFFFA, 0xA4506CEB, 0xBEF9A3F7, 0xC67178F2,
];
const SHA256_ADJUSTMENT: u32 = 0xC9DC06A7;
const SHA256_SIGMA0_ADJUSTMENT: u32 = 0x059C0B80;
const SHA256_SIGMA1_ADJUSTMENT: u32 = 0x000B92C0;

const CS64_COEFFICIENTS: [(u32, u32); 14] = [
    (0xE82E91B1, 0xB32700D0),
    (0x44DE24C6, 0x88DFF51D),
    (0x3BEB65BC, 0x29514038),
    (0xC89C5933, 0xB328CBDF),
    (0x8D1EC211, 0xD405B4D5),
    (0x314EF5D5, 0x26E55F3C),
    (0xE92491B1, 0x607801C6),
    (0xF0A5B9D5, 0x82B2C1FB),
    (0x06064CF3, 0xFA51A3DA),
    (0xDA79FA60, 0x207CC464),
    (0xBED20560, 0x3C3C74F3),
    (0x89F9C9F8, 0xD2D14B61),
    (0x35FA38C6, 0xBB04A2C1),
    (0xCB49E3C6, 0xC5626167),
];
const CS64_FACTOR: u32 = 0x82893933;
const CS64_ADJUSTMENT0: u32 = 0xF2F90B1A;
const CS64_ADJUSTMENT1: u32 = 0x0B1A0000;

/// 32 位值循环左移 16 位
fn rol16(value: u32) -> u32 {
    value.rotate_left(16)
}

/// 小端 32 位字拆分，零填充到偶数个
fn words_le4_even(data: &[u8]) -> Vec<u32> {
    let pad = (4 - (data.len() % 4)) % 4;
    let mut padded = data.to_vec();
    padded.extend(std::iter::repeat(0u8).take(pad));
    let mut words: Vec<u32> = padded
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if words.len() % 2 == 1 {
        words.push(0);
    }
    words
}

/// SHA-256 小 sigma0（移位组合，等价 rotr7 ^ rotr18 ^ shr3）
fn sigma0(value: u32) -> u32 {
    (value >> 7) ^ (value << 25) ^ (value >> 18) ^ (value << 14) ^ (value >> 3)
}

/// SHA-256 小 sigma1（等价 rotr17 ^ rotr19 ^ shr10）
fn sigma1(value: u32) -> u32 {
    (value >> 17) ^ (value << 15) ^ (value >> 19) ^ (value << 13) ^ (value >> 10)
}

fn rotr(v: u32, n: u32) -> u32 {
    v.rotate_right(n)
}

/// SHA-256 轮函数；first_round=14 时（init 用）第 14 个轮常量替换为 ADJUSTMENT
fn sha256_rounds(state: [u32; 8], words: &[u32], first_round: usize) -> [u32; 8] {
    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];
    for i in first_round..64 {
        let k = if first_round == 14 && i == 14 {
            SHA256_ADJUSTMENT
        } else {
            SHA256_ROUND_CONSTANTS[i]
        };
        // Ch(e,f,g) = g ^ (e & (f ^ g))；Maj(a,b,c) = (a & b) | (c & (a | b))
        let t1 = h
            .wrapping_add(rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25))
            .wrapping_add(g ^ (e & (f ^ g)))
            .wrapping_add(k)
            .wrapping_add(words[i]);
        let t2 = (rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)).wrapping_add((a & b) | (c & (a | b)));
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    [a, b, c, d, e, f, g, h]
}

/// 消息调度扩展（edge-tts 版 sigma 表达式）
fn expand_schedule(words: &mut Vec<u32>, first_word: usize) {
    for i in first_word..64 {
        let x = words[i - 2];
        let y = words[i - 15];
        let s1 = (x >> 17) ^ (x << 15) ^ (x >> 19) ^ (x << 13) ^ (x >> 10);
        let s0 = (y >> 7) ^ (y << 25) ^ (y >> 18) ^ (y << 14) ^ (y >> 3);
        words.push(
            s1.wrapping_add(words[i - 7])
                .wrapping_add(s0)
                .wrapping_add(words[i - 16]),
        );
    }
}

/// 定制 SHA-256 初始状态（基于 token 盐前 7 字节）
fn sha256_init(data: &[u8]) -> [u32; 8] {
    let mut words: Vec<u32> = vec![0; 16];
    words[14] = u32::from_be_bytes([0, data[0], data[1], data[2]]);
    words[15] = u32::from_be_bytes([data[3], data[4], data[5], data[6]]);
    for index in 16..30 {
        let mut value = SHA256_SCHEDULE[index - 16];
        if index - 2 >= 14 {
            value = value.wrapping_add(if index == 16 {
                sigma1(words[index - 2]) ^ SHA256_SIGMA1_ADJUSTMENT
            } else {
                sigma1(words[index - 2])
            });
        }
        if index - 7 >= 14 {
            value = value.wrapping_add(words[index - 7]);
        }
        if index - 15 >= 14 {
            value = value.wrapping_add(if index == 29 {
                sigma0(words[index - 15]) ^ SHA256_SIGMA0_ADJUSTMENT
            } else {
                sigma0(words[index - 15])
            });
        }
        words.push(value);
    }
    let w30 = sigma1(words[28])
        .wrapping_add(words[23])
        .wrapping_add(sigma0(words[15]))
        .wrapping_add(words[14])
        .wrapping_add(SHA256_ADJUSTMENT)
        .wrapping_sub(SHA256_ROUND_CONSTANTS[14]);
    words.push(w30);
    expand_schedule(&mut words, 31);
    let working = sha256_rounds(SHA256_STATE, &words, 14);
    let mut out = [0u32; 8];
    for i in 0..8 {
        out[i] = SHA256_INITIAL[i].wrapping_add(working[i]);
    }
    out
}

/// 定制 SHA-256 摘要（token 数据专用）
fn sha256_digest(data: &[u8]) -> [u8; 32] {
    let length = 57 + data.len();
    let mut padded = data[7..].to_vec();
    padded.push(0x80);
    padded.extend(std::iter::repeat(0u8).take((55usize).wrapping_sub(length) % 64));
    padded.extend_from_slice(&((length as u64) * 8).to_be_bytes());
    let mut state = sha256_init(&data[..7]);
    for chunk in padded.chunks_exact(64) {
        let mut words: Vec<u32> = chunk
            .chunks_exact(4)
            .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        expand_schedule(&mut words, 16);
        let working = sha256_rounds(state, &words, 0);
        for i in 0..8 {
            state[i] = state[i].wrapping_add(working[i]);
        }
    }
    let mut out = [0u8; 32];
    for (i, w) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    out
}

/// CS64 摘要（两个 32 位字）
fn cs64_digest(data: &[u8], digest: &[u8; 32]) -> (u32, u32) {
    let key_a = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]) | 1;
    let key_b = u32::from_le_bytes([digest[4], digest[5], digest[6], digest[7]]) | 1;
    let f0 = key_a.wrapping_mul(CS64_FACTOR);
    let f1 = key_b.wrapping_mul(CS64_FACTOR);
    let mut terms: Vec<u32> = CS64_COEFFICIENTS
        .iter()
        .enumerate()
        .map(|(i, (c, _))| c.wrapping_mul(if i % 2 == 0 { f0 } else { f1 }))
        .collect();
    let mut offsets: Vec<u32> = CS64_COEFFICIENTS.iter().map(|(_, o)| *o).collect();

    let mut input = Vec::with_capacity(data.len() + 1);
    input.push(0u8);
    input.extend_from_slice(data);
    for (index, word) in words_le4_even(&input).into_iter().enumerate() {
        let mut term = word.wrapping_mul(if index % 2 == 0 { key_a } else { key_b });
        let mut offset =
            rol16(word).wrapping_mul(if index % 2 == 0 { 0xEF0569FB } else { 0xC31713DB });
        if index == 0 {
            term = term.wrapping_add(CS64_ADJUSTMENT0.wrapping_mul(f0));
            offset = offset.wrapping_add(CS64_ADJUSTMENT1);
        }
        terms.push(term);
        offsets.push(offset);
    }

    let mut swap_acc: u32 = 0;
    let mut swap_last: u32 = 0;
    let mut state: u32 = 0;
    let mut acc: u32 = 0;
    let mut i = 0;
    while i + 1 < terms.len() {
        let x = terms[i];
        let y = terms[i + 1];
        let mut t = rol16(offsets[i].wrapping_add(x));
        let mut q = t.wrapping_mul(0x76F50F34);
        swap_acc = swap_acc.wrapping_add(rol16(q).wrapping_mul(0xC3EFEA97).wrapping_add(q));
        t = rol16(offsets[i + 1].wrapping_add(y));
        q = t.wrapping_mul(0x1881CE3C);
        swap_last = rol16(q).wrapping_mul(0x35BD1EC9).wrapping_add(q);
        swap_acc = swap_acc.wrapping_add(swap_last);

        state = state.wrapping_mul(key_a).wrapping_add(x);
        state = rol16(state).wrapping_mul(0xCF98B111);
        state = rol16(state).wrapping_mul(0x87085B9F);
        state = rol16(state).wrapping_mul(0x12CEB96D);
        state = rol16(state).wrapping_mul(0x257E1D83);
        acc = acc.wrapping_add(state);
        let mut t2 = state.wrapping_mul(key_b).wrapping_add(y);
        t2 = rol16(t2).wrapping_mul(0xA27416F5);
        t2 = rol16(t2).wrapping_mul(0xD38396FF);
        state = rol16(rol16(t2).wrapping_mul(0x7C932B89)).wrapping_mul(0xBFA49F69);
        acc = acc.wrapping_add(state);
        i += 2;
    }
    (swap_last ^ state, swap_acc ^ acc)
}

/// v2 token：`{cs64}-{sha256}`（16hex-64hex）
fn token_hash(data: &[u8]) -> String {
    let digest = sha256_digest(data);
    let (c0, c1) = cs64_digest(data, &digest);
    let mut cs64 = [0u8; 8];
    cs64[..4].copy_from_slice(&c0.to_le_bytes());
    cs64[4..].copy_from_slice(&c1.to_le_bytes());
    let cs64_hex: String = cs64.iter().map(|b| format!("{b:02X}")).collect();
    let sha_hex: String = digest.iter().map(|b| format!("{b:02X}")).collect();
    format!("{cs64_hex}-{sha_hex}")
}

/// unix 秒 → Sec-MS-GEC v2：加 Windows 纪元，向下取整到 5 分钟，转 100ns ticks，
/// 拼 TrustedClientToken 后做定制 SHA-256 + CS64 双摘要，输出 `{cs64}-{sha256}`
pub fn sec_ms_gec(unix_seconds: f64) -> String {
    let mut ticks = unix_seconds + WIN_EPOCH;
    ticks -= ticks % 300.0;
    ticks *= 10_000_000.0;
    let to_hash = format!("{:.0}{}", ticks, TRUSTED_CLIENT_TOKEN);
    token_hash(to_hash.as_bytes())
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
        // tungstenite 走 http::Request 路径时要求调用者自带全部握手 header
        .header("Host", "speech.platform.bing.com")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            base64::engine::general_purpose::STANDARD.encode(uuid::Uuid::new_v4().as_bytes()),
        )
        .header("Origin", "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold")
        .header("User-Agent", UA)
        .header("Pragma", "no-cache")
        .header("Cache-Control", "no-cache")
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
        // 期望值由 edge-tts drm.generate_sec_ms_gec_v2 同算法 + 固定时间戳算出
        assert_eq!(
            sec_ms_gec(1_770_000_000.0),
            "00DDB33FD01602D8-87C1FE6B212DC02BD1C09F05EE72850D2069A3CC92C5F462FDD6DDE2B15E8B08"
        );
        // 同一时刻幂等
        assert_eq!(sec_ms_gec(1_770_000_000.0), sec_ms_gec(1_770_000_000.0));
        // 随机时间戳回归（Python 参考实现生成）
        assert_eq!(
            sec_ms_gec(1763942679.8457885),
            "8C5326258B238311-A4E7A2E3B4B2360E354C76C8EC67FDB97943D8A0F54878F52CCECD7385732214"
        );
        assert_eq!(
            sec_ms_gec(1702501075.5222666),
            "CDBE17CD38DE7F1B-FC5C538764D5D6EAB809919FC5A6097B93DBF463DBC34EF86E3DA77B5F311C5D"
        );
        assert_eq!(
            sec_ms_gec(1773647121.4164011),
            "8245ED42AC82EB9E-3A1BA6156DEE2E303065C06F163EECDC88F1466E386BC3C7152E20EB0450A82F"
        );
        // v2 格式：16hex-64hex
        let tok = sec_ms_gec(1_770_000_000.0);
        assert_eq!(tok.len(), 81);
        assert!(tok.contains('-'));
        assert!(tok.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
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

    /// 真实联网集成测试：完整协议走一遍微软 Edge TTS 服务
    /// 断言返回有效 mp3（ID3 头或 MPEG 帧同步字节），体积 > 1KB
    #[tokio::test]
    async fn tts_live_synthesize() {
        let dir = std::env::temp_dir().join(format!("mindpal-tts-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("live.mp3");
        synthesize("你好，这是一次语音合成测试。", "zh-CN-XiaoxiaoNeural", &out)
            .await
            .unwrap();
        let data = std::fs::read(&out).unwrap();
        assert!(data.len() > 1024, "音频太小: {} 字节", data.len());
        let is_mp3 = data.starts_with(b"ID3") || (data[0] == 0xFF && (data[1] & 0xE0) == 0xE0);
        assert!(is_mp3, "不是有效 mp3: 前 4 字节 {:?}", &data[..4.min(data.len())]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
