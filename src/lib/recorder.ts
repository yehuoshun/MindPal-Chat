/**
 * 麦克风录音：getUserMedia → ScriptProcessor 采集 Float32 PCM → 停止时转 i16 + base64
 * 采样率交给后端重采样到 16kHz（Whisper 要求）
 * 纯数据处理函数在 ./audio（单独单测）
 */

import { floatToI16, mergeFloat32 } from "./audio";

export interface RecordingResult {
  pcm: Int16Array;
  sampleRate: number;
}

export class Recorder {
  private ctx: AudioContext | null = null;
  private stream: MediaStream | null = null;
  private node: ScriptProcessorNode | null = null;
  private source: MediaStreamAudioSourceNode | null = null;
  private chunks: Float32Array[] = [];

  async start(): Promise<void> {
    if (this.ctx) return;
    this.stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    });
    const Ctor: typeof AudioContext =
      window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    const ctx = new Ctor();
    const source = ctx.createMediaStreamSource(this.stream);
    // ScriptProcessor 已废弃但各 WebView 都支持，且免 AudioWorklet 额外模块
    const node = ctx.createScriptProcessor(4096, 1, 1);
    this.chunks = [];
    node.onaudioprocess = (e) => {
      this.chunks.push(new Float32Array(e.inputBuffer.getChannelData(0)));
    };
    source.connect(node);
    node.connect(ctx.destination); // 不连 destination 部分实现不触发回调
    this.ctx = ctx;
    this.source = source;
    this.node = node;
  }

  async stop(): Promise<RecordingResult> {
    const sampleRate = this.ctx?.sampleRate ?? 48000;
    this.node?.disconnect();
    this.source?.disconnect();
    this.node = null;
    this.source = null;
    this.stream?.getTracks().forEach((t) => t.stop());
    this.stream = null;
    await this.ctx?.close().catch(() => {});
    this.ctx = null;

    const all = mergeFloat32(this.chunks);
    this.chunks = [];
    return { pcm: floatToI16(all), sampleRate };
  }

  cancel() {
    this.node?.disconnect();
    this.source?.disconnect();
    this.stream?.getTracks().forEach((t) => t.stop());
    void this.ctx?.close().catch(() => {});
    this.node = null;
    this.source = null;
    this.stream = null;
    this.ctx = null;
    this.chunks = [];
  }
}
