import { describe, expect, it } from "vitest";
import { floatToI16, mergeFloat32, pcmToBase64 } from "./audio";

describe("mergeFloat32", () => {
  it("按顺序拼接且长度正确", () => {
    const out = mergeFloat32([new Float32Array([1, 2]), new Float32Array([3]), new Float32Array([])]);
    expect(Array.from(out)).toEqual([1, 2, 3]);
  });

  it("空输入返回空数组", () => {
    expect(mergeFloat32([]).length).toBe(0);
  });
});

describe("floatToI16", () => {
  it("端点与中点转换正确", () => {
    const out = floatToI16(new Float32Array([0, 1, -1, 0.5, -0.5]));
    expect(Array.from(out)).toEqual([0, 32767, -32768, 16383, -16384]);
  });

  it("超界钳制（防爆音）", () => {
    const out = floatToI16(new Float32Array([2, -2, 100]));
    expect(Array.from(out)).toEqual([32767, -32768, 32767]);
  });

  it("长度保持一致", () => {
    expect(floatToI16(new Float32Array(1000)).length).toBe(1000);
  });
});

describe("pcmToBase64", () => {
  const decode = (b64: string): number[] => Array.from(atob(b64), (c) => c.charCodeAt(0));

  it("小端字节序往返正确", () => {
    // 0x0100 = 256（小端 → 00 01）
    const pcm = new Int16Array([256, -1]);
    expect(decode(pcmToBase64(pcm))).toEqual([0x00, 0x01, 0xff, 0xff]);
  });

  it("大输入分块不爆栈且内容完整", () => {
    const n = 20000; // > 0x2000 分块阈值
    const pcm = new Int16Array(n);
    for (let i = 0; i < n; i++) pcm[i] = (i % 1000) - 500;
    const bytes = decode(pcmToBase64(pcm));
    expect(bytes.length).toBe(n * 2);
    const back = new Int16Array(new Uint8Array(bytes).buffer);
    expect(Array.from(back)).toEqual(Array.from(pcm));
  });

  it("空输入得到空串", () => {
    expect(pcmToBase64(new Int16Array(0))).toBe("");
  });
});