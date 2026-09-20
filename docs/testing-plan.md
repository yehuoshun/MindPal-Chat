# MindPal Chat — 测试方案

## CI 矩阵

| 端 | GitHub Action Runner | 能跑什么测试 |
|----|:-------------------:|:------------|
| **Linux** | `ubuntu-latest` | 所有（单元测试 + 构建 + Web 端测试） |
| **Windows** | `windows-latest` | 所有 + .exe 打包验证 |
| **macOS** | `macos-latest` | 单元测试 + .dmg 构建（iOS unit test 可跑） |
| **Web** | `ubuntu-latest`（复用） | npm test / Vitest / Cypress E2E |
| **Android** | `ubuntu-latest` + `android-emulator-runner` | 单元测试 + 模拟器 UI 测试 |
| **iOS** | `macos-latest` | ⚠️ 只能跑 **unit test** |

**跑不了的**：
- iOS 模拟器 UI 集成测试（需要 Mac 物理机 + Xcode GUI 加速）
- 连接物理真机的测试

---

## 测试分层

```
分层              工具                 覆盖内容
──────────────────────────────────────────────────
Lint             ESLint / Prettier    代码风格、语法错误
Unit Test        Vitest / Jest        组件逻辑、Rust 函数、LLM 路由逻辑
Component Test   Vitest + Testing Library  UI 组件渲染与交互
Integration Test Playwright / Cypress  用户操作全流程（发消息、切换人格、语音按钮）
Build Test       cargo tauri build     Win/Linux/macOS 三端能否编译通过
Android E2E      @reactivecircus/android-emulator-runner   Android 模拟器 UI 测试
```

---

## CI Workflow 设计

### PR 阶段（每次 push）

```yaml
name: PR Check

on: [pull_request]

jobs:
  lint-and-unit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: npm ci
      - run: npm run lint
      - run: npm run test:unit

  build:
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - run: npm ci
      - run: npm run tauri build
```

### Web E2E 测试

```yaml
web-e2e:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - run: npm ci
    - run: npm run dev & npx wait-on http://localhost:5173
    - run: npx cypress run
```

### Android 模拟器测试

```yaml
android-test:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: reactivecircus/android-emulator-runner@v2
      with:
        api-level: 34
        script: |
          cd src-tauri
          cargo test --target aarch64-linux-android
```

### Release 阶段（打 tag 时）

```yaml
release:
  strategy:
    matrix:
      os: [ubuntu-latest, windows-latest, macos-latest]
  runs-on: ${{ matrix.os }}
  steps:
    - run: npm ci
    - run: npm run tauri build
    - uses: softprops/action-gh-release@v2
      with:
        files: src-tauri/target/release/bundle/*
```

---

## 关键注意事项

1. **iOS 真机测试**：必须有 Mac + Apple Developer 账号 + Xcode，CI 跑不了，需本地或第三方服务
2. **Android 模拟器**：CI 能跑，但很慢（5-15 分钟），建议只在 main 分支 merge 时跑
3. **API 依赖**：LLM 测试需要 mock API，不要每次都真调用（花钱又慢）
4. **语音测试**：TTS/STT 依赖音频文件，测试用固定测试音频
5. **隐私检查**：CI 中配置 `git diff --cached | grep -iE '(token|secret|key|password)'` 检查，避免 Token