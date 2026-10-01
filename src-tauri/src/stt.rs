//! 本地语音识别（STT）—— whisper.cpp（whisper-rs 绑定）
//!
//! 流程：前端录音（PCM i16）→ base64 → 本模块解码/重采样到 16kHz → whisper.cpp 本地识别 → 文本
//! 模型：首次使用需下载 ggml 模型（HuggingFace），存 app_data/models/，本模块带进度事件
//! 平台：桌面端启用；Android 交叉编译 whisper.cpp 留二期（Cargo.toml 里 target 隔离）

use base64::Engine;
use futures_util::StreamExt;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// HuggingFace 上 whisper.cpp 官方模型库
pub const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";
/// whisper.cpp 要求的采样率
pub const TARGET_SAMPLE_RATE: u32 = 16_000;
/// 默认模型
pub const DEFAULT_MODEL: &str = "base";

/// 可选模型：(键, 文件名, 约体积 MB)
pub const MODELS: &[(&str, &str, u32)] = &[
    ("tiny", "ggml-tiny-q5_1.bin", 32),
    ("base", "ggml-base-q5_1.bin", 60),
    ("small", "ggml-small-q5_1.bin", 190),
];

#[derive(Debug, Clone, Serialize)]
pub struct SttStatus {
    pub supported: bool,
    pub model: String,
    pub model_present: bool,
    pub model_path: String,
    pub size_mb: u32,
}

// ---------- 平台能力 ----------

#[cfg(not(target_os = "android"))]
pub fn platform_supported() -> bool {
    true
}

#[cfg(target_os = "android")]
pub fn platform_supported() -> bool {
    false
}

/// 已加载的 whisper 上下文类型（Android 下为占位，保持 AppState 统一）
#[cfg(not(target_os = "android"))]
pub type SharedContext = std::sync::Arc<whisper_rs::WhisperContext>;

#[cfg(target_os = "android")]
pub type SharedContext = ();

// ---------- 模型映射（纯函数，可测） ----------

pub fn model_file(size: &str) -> Option<&'static str> {
    MODELS.iter().find(|(k, _, _)| *k == size).map(|(_, f, _)| *f)
}

pub fn model_url(size: &str) -> Option<String> {
    model_file(size).map(|f| format!("{HF_BASE}/{f}"))
}

pub fn model_size_mb(size: &str) -> u32 {
    MODELS.iter().find(|(k, _, _)| *k == size).map(|(_, _, m)| *m).unwrap_or(0)
}

/// 未知模型名回退默认
pub fn normalize_model(size: &str) -> &str {
    match model_file(size) {
        Some(_) => size,
        None => DEFAULT_MODEL,
    }
}

pub fn model_path(dir: &Path, size: &str) -> PathBuf {
    dir.join(model_file(normalize_model(size)).unwrap_or("ggml-base-q5_1.bin"))
}

// ---------- 音频转换（纯函数，可测） ----------

pub fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("音频解码失败: {e}"))
}

/// 小端 i16 字节流 → i16 采样
pub fn bytes_to_i16(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// i16 → f32（归一化到 [-1, 1]）
pub fn pcm_i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|s| *s as f32 / 32768.0).collect()
}

/// 重采样到目标采样率：
/// - 整数倍降采样用均值（充当低通，避免混叠）
/// - 其他比例用线性插值
pub fn resample_linear(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 || to == 0 {
        return input.to_vec();
    }
    if from % to == 0 {
        let factor = (from / to) as usize;
        return input
            .chunks(factor)
            .filter(|c| !c.is_empty())
            .map(|c| c.iter().sum::<f32>() / c.len() as f32)
            .collect();
    }
    let ratio = from as f64 / to as f64;
    let out_len = ((input.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let a = input[idx];
        let b = if idx + 1 < input.len() { input[idx + 1] } else { a };
        out.push(a + (b - a) * frac);
    }
    out
}

// ---------- 模型下载（可测：mock server） ----------

/// 流式下载到 dest（先写 .part 再改名），过程中回调 (已下载, 总大小)
pub async fn download_to(
    url: &str,
    dest: &Path,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败 HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建模型目录失败: {e}"))?;
    }
    let tmp = dest.with_extension("part");
    let mut file = std::fs::File::create(&tmp).map_err(|e| format!("创建文件失败: {e}"))?;
    use std::io::Write;
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|e| format!("下载中断: {e}"))?;
        file.write_all(&chunk).map_err(|e| format!("写入失败: {e}"))?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total);
    }
    file.flush().ok();
    drop(file);
    std::fs::rename(&tmp, dest).map_err(|e| format!("保存模型失败: {e}"))?;
    Ok(())
}

pub fn models_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("models")
}

// ---------- 引擎（桌面端） ----------

#[cfg(not(target_os = "android"))]
pub mod engine {
    use super::SharedContext;
    use std::path::Path;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    pub fn load_context(path: &Path) -> Result<SharedContext, String> {
        WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map(std::sync::Arc::new)
            .map_err(|e| format!("加载语音模型失败: {e}"))
    }

    pub fn transcribe(
        ctx: &WhisperContext,
        samples: &[f32],
        language: Option<&str>,
    ) -> Result<String, String> {
        let mut state = ctx
            .create_state()
            .map_err(|e| format!("创建识别状态失败: {e}"))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_language(language);
        params.set_n_threads(threads());
        state
            .full(params, samples)
            .map_err(|e| format!("语音识别失败: {e}"))?;
        let mut out = String::new();
        for seg in state.as_iter() {
            out.push_str(seg.to_str().unwrap_or(""));
        }
        Ok(out.trim().to_string())
    }

    fn threads() -> i32 {
        std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4)
            .min(8)
    }
}

// ---------- Tauri 命令 ----------

#[tauri::command]
pub async fn stt_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<SttStatus, String> {
    use tauri::Manager;
    let dir = models_dir(&app.path().app_data_dir().map_err(|e| e.to_string())?);
    let model = normalize_model(&state.config.lock().unwrap().stt_model).to_string();
    let path = model_path(&dir, &model);
    Ok(SttStatus {
        supported: platform_supported(),
        model: model.clone(),
        model_present: path.exists(),
        model_path: path.to_string_lossy().to_string(),
        size_mb: model_size_mb(&model),
    })
}

#[tauri::command]
pub async fn stt_download_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    size: String,
) -> Result<String, String> {
    use tauri::{Emitter, Manager};
    if !platform_supported() {
        return Err("当前平台暂不支持本地语音识别".to_string());
    }
    let size = normalize_model(&size).to_string();
    let dir = models_dir(&app.path().app_data_dir().map_err(|e| e.to_string())?);
    let path = model_path(&dir, &size);
    let url = model_url(&size).ok_or_else(|| "未知模型".to_string())?;

    let app2 = app.clone();
    download_to(&url, &path, move |downloaded, total| {
        let _ = app2.emit(
            "stt-download",
            serde_json::json!({ "downloaded": downloaded, "total": total }),
        );
    })
    .await?;

    // 换了模型，清掉已加载上下文
    *state.whisper.lock().unwrap() = None;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn stt_transcribe(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    pcm_base64: String,
    sample_rate: u32,
    language: Option<String>,
) -> Result<String, String> {
    transcribe_impl(app, state, pcm_base64, sample_rate, language).await
}

#[cfg(target_os = "android")]
async fn transcribe_impl(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, crate::AppState>,
    _pcm_base64: String,
    _sample_rate: u32,
    _language: Option<String>,
) -> Result<String, String> {
    Err("当前平台暂不支持本地语音识别".to_string())
}

#[cfg(not(target_os = "android"))]
async fn transcribe_impl(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    pcm_base64: String,
    sample_rate: u32,
    language: Option<String>,
) -> Result<String, String> {
    use tauri::Manager;

    let bytes = decode_base64(&pcm_base64)?;
    let i16s = bytes_to_i16(&bytes);
    // 太短的音频（<0.3s）直接拒绝
    if i16s.len() < (TARGET_SAMPLE_RATE as usize * 3) / 10 {
        return Err("录音太短，请再说一次".to_string());
    }
    let f32s = pcm_i16_to_f32(&i16s);
    let samples = if sample_rate == TARGET_SAMPLE_RATE {
        f32s
    } else {
        resample_linear(&f32s, sample_rate.max(1), TARGET_SAMPLE_RATE)
    };

    let dir = models_dir(&app.path().app_data_dir().map_err(|e| e.to_string())?);
    let model = normalize_model(&state.config.lock().unwrap().stt_model).to_string();
    let path = model_path(&dir, &model);
    if !path.exists() {
        return Err(format!(
            "语音模型未下载（{model} 约 {}MB），请到设置 → 语音输入里下载",
            model_size_mb(&model)
        ));
    }

    // 复用已加载上下文（加载一次较慢，放阻塞线程池）
    let cached = state.whisper.lock().unwrap().clone();
    let ctx = match cached {
        Some(c) => c,
        None => {
            let p = path.clone();
            let c = tauri::async_runtime::spawn_blocking(move || engine::load_context(&p))
                .await
                .map_err(|e| format!("模型加载任务失败: {e}"))??;
            *state.whisper.lock().unwrap() = Some(c.clone());
            c
        }
    };

    let lang = language.unwrap_or_else(|| "zh".to_string());
    tauri::async_runtime::spawn_blocking(move || {
        engine::transcribe(&ctx, &samples, Some(lang.as_str()))
    })
    .await
    .map_err(|e| format!("识别任务失败: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn model_mapping() {
        assert_eq!(model_file("base"), Some("ggml-base-q5_1.bin"));
        assert!(model_url("tiny").unwrap().ends_with("/ggml-tiny-q5_1.bin"));
        assert_eq!(model_url("nope"), None);
        assert_eq!(normalize_model("nope"), DEFAULT_MODEL);
        assert_eq!(normalize_model("small"), "small");
        assert_eq!(model_size_mb("base"), 60);
        assert_eq!(model_size_mb("nope"), 0);
        assert_eq!(
            model_path(Path::new("/tmp/x"), "tiny"),
            Path::new("/tmp/x/ggml-tiny-q5_1.bin")
        );
        assert_eq!(
            model_path(Path::new("/tmp/x"), "bad"),
            Path::new("/tmp/x/ggml-base-q5_1.bin")
        );
    }

    #[test]
    fn pcm_conversion() {
        assert_eq!(pcm_i16_to_f32(&[0])[0], 0.0);
        assert!((pcm_i16_to_f32(&[32767])[0] - 0.99997).abs() < 0.001);
        assert_eq!(pcm_i16_to_f32(&[-32768])[0], -1.0);

        let samples: Vec<i16> = vec![-1, 0, 1, 1000, -1000];
        let mut bytes = Vec::new();
        for s in &samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        assert_eq!(bytes_to_i16(&bytes), samples);
        // 奇数长度字节流忽略尾巴
        assert_eq!(bytes_to_i16(&[1, 0, 2]).len(), 1);
    }

    #[test]
    fn base64_roundtrip_and_errors() {
        let raw: Vec<u8> = vec![1, 2, 3, 4, 250];
        let enc = base64::engine::general_purpose::STANDARD.encode(&raw);
        assert_eq!(decode_base64(&enc).unwrap(), raw);
        assert!(decode_base64("!!!not base64!!!").is_err());
    }

    #[test]
    fn resample_downsample_by_average() {
        // 48k → 16k：3:1 均值
        let input: Vec<f32> = (0..30).map(|i| i as f32).collect();
        let out = resample_linear(&input, 48000, 16000);
        assert_eq!(out.len(), 10);
        assert_eq!(out[0], 1.0); // (0+1+2)/3
        assert_eq!(out[9], 28.0); // (27+28+29)/3
        // 恒定信号降采样后不变
        let dc = vec![0.5f32; 480];
        assert!(resample_linear(&dc, 48000, 16000).iter().all(|v| (*v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn resample_identity_and_empty() {
        let input = vec![0.1f32, 0.2, 0.3];
        assert_eq!(resample_linear(&input, 16000, 16000), input);
        assert!(resample_linear(&[], 48000, 16000).is_empty());
        // 非整数倍：44100 → 16000 长度约 0.363 倍
        let out = resample_linear(&vec![0.0f32; 4410], 44100, 16000);
        assert!(out.len() >= 1595 && out.len() <= 1605, "len={}", out.len());
    }

    #[tokio::test]
    async fn download_writes_file_and_reports_progress() {
        let payload: &'static [u8] = b"fake-model-bytes-0123456789";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok(mut stream) = listener.accept().map(|(s, _)| s) {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(payload);
            }
        });

        let dir = std::env::temp_dir().join(format!("mindpal-stt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("model.bin");
        let mut last: (u64, u64) = (0, 0);
        download_to(&format!("http://{addr}/model.bin"), &dest, |d, t| last = (d, t))
            .await
            .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), payload);
        assert_eq!(last.0, payload.len() as u64);
        assert_eq!(last.1, payload.len() as u64);
        assert!(!dest.with_extension("part").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn download_reports_http_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok(mut stream) = listener.accept().map(|(s, _)| s) {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
        let dir = std::env::temp_dir().join(format!("mindpal-stt-404-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = download_to(&format!("http://{addr}/x"), &dir.join("m.bin"), |_, _| {})
            .await
            .unwrap_err();
        assert!(err.contains("404"), "err={err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}