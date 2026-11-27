# TYUT Keepalive

太原理工大学校园网自动认证与保活桌面工具，基于 Rust 和 Tauri 2。官方仓库：[NoRegret-L/tyut-keepalive](https://github.com/NoRegret-L/tyut-keepalive)。

> v0.5.0 为待发布开发版本，当前正式版以 Releases 为准。PR 与主分支执行 Windows、macOS、Linux 三平台 CI；正式发布仍需通过签名及真机验收。

## 安装与使用

从官方 [Releases](https://github.com/NoRegret-L/tyut-keepalive/releases) 下载实际已发布的对应平台安装包。旧版发布资产的名称可能不同，不要把开发分支当成已发布安装包。

| 平台 | 计划提供的安装包 | 注意事项 |
| --- | --- | --- |
| Windows x64 | MSI | 按安装器提示安装，需 WebView2 |
| macOS Universal（Intel / Apple Silicon，开发版目标） | DMG | 拖入应用程序目录；实际兼容性和系统签名仍需验证 |
| Linux x64 | DEB、AppImage | DEB 使用系统包管理器升级；AppImage 可使用签名更新 |

1. 打开应用，在连接设置中输入校园网账号和密码，保存凭据。
2. 概览页查看实时状态、最近检测和认证时间、观测中断及在线时长。
3. 设置页可切换浅色、深色及系统主题，开启自启和系统通知（通知默认关闭）。
4. 关闭窗口会隐藏至托盘；无托盘时降级为最小化。彻底退出才结束后台服务，自启需单独关闭。
5. 暂停保活不会注销当前网络，且仅对本次运行有效；唤醒和网络变化不会自动解除暂停。密码被拒绝后自动重试停止。

## 日志、凭据与安全

- 凭据保存在系统密钥库：Windows Credential Manager、macOS Keychain、Linux Secret Service。旧版应用数据目录中的 config.enc 在安全迁移并校验成功后删除；不会在迁移失败时自动回退到不安全存储。
- 凭据清除失败时保存不含秘密的清除标记，阻止下次启动自动使用未删除的凭据；再次清除或明确保存新凭据可恢复。若应用数据目录本身不可写，无法保证标记落盘，需手工确认密钥库记录。
- 日志位于 Tauri 应用数据目录的 telemetry 子目录；两个约 1 MiB 的 JSONL 文件轮转，界面显示最近 200 条。日志不保存账号、密码、IP、MAC 或认证请求 URL。
- 统计是程序的网络观察结果，不等于校方服务器的登录会话统计。原始校园网探测规则可能误判。Linux 电源监听依赖 systemd-logind。
- 后端统一处理网络变化、休眠、暂停和恢复，防止并发认证及过期任务结果覆盖状态。
- 仅本地主窗口获准调用必要 IPC；不授权 WebView 通用通知、更新、文件系统、Shell 或远程事件发送权限。CSP 禁止远程脚本。RustSec 审计及配置检查进入 CI。
- 校园 Portal 探测仍使用校方既有 HTTP 协议；认证请求使用 HTTPS。密钥库、真实校园网、托盘、通知及休眠恢复需要实机验证。

## 签名更新

关于页在配置有效公钥后启动检查一次，并提供手动检查；安装必须由用户确认。官方 updater 使用 HTTPS、Minisign 签名和签名版本绑定，禁止降级。未配置公钥时更新停用，构建仍可用于开发。DEB 等系统包管理器安装方式不保证应用内更新。

发布维护者在可信本机运行：

```sh
npm ci
npm run tauri -- signer generate -w ~/.tauri/tyut-keepalive.key
```

仅将 .pub 文件内容填入 src-tauri/tauri.conf.json 的 plugins.updater.pubkey。私钥和密码应保存于 GitHub Actions Secrets（TAURI_SIGNING_PRIVATE_KEY、TAURI_SIGNING_PRIVATE_KEY_PASSWORD），不得提交或发送给他人。丢失私钥可能使既有安装无法安全升级。更新签名不是 Windows Authenticode 或 macOS Developer ID 签名的替代品。

正式发布前须统一 package.json、package-lock.json、Cargo.toml、Cargo.lock、tauri.conf.json 中的应用版本，并由维护者确认发布标签。发布工作流先完成三平台严格锁定测试、RustSec 审计和公钥/标签检查，再签名构建上传草稿；更新清单包含 Windows、Linux 及 macOS 双架构后才公开 Release。当前未配置公钥、未创建标签、未进行真实签名升级测试。

## 源码开发

安装 Rust stable、Node.js 24、Python 3.11+ 及 [Tauri 平台依赖](https://v2.tauri.app/start/prerequisites/)。

Ubuntu/Debian 参考依赖：

```sh
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev
```

```sh
npm ci
npm run tauri -- dev
npm run check:html
node --test scripts/test_ui.mjs
python scripts/check_security.py
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo check --manifest-path src-tauri/Cargo.toml --release --locked
cargo install cargo-audit --version 0.23.0 --locked
cargo audit --file src-tauri/Cargo.lock
```

开发检查采用锁定依赖；新增依赖的提交阶段可临时由开发 CI 生成 Cargo.lock 产物，维护者校验提交后须恢复严格锁定。正式发布始终严格锁定。

开发设计记录见 [docs](docs/)。

发布前检查与剩余依赖风险见 [v0.4.0 验收记录](docs/v0.4.0-release-review.md)。

## 独立测试安装包

开发分支的 [Independent Test Installers](https://github.com/NoRegret-L/tyut-keepalive/actions/workflows/test-build.yml) 默认仅生成 Actions Artifacts；维护者可在官方仓库手动运行工作流发布按提交 SHA 标识的独立 prerelease，不覆盖旧资产、不设为 latest。选择成功运行的 Windows MSI、Linux DEB/AppImage 或 macOS Universal DMG；压缩包内含 SHA256SUMS.txt 和 BUILD-INFO.json。测试版独立安装、凭据及自启，不启用更新，不配置系统代码签名。详细验收步骤见 [独立测试版说明](docs/v0.4.0-test-build.md)。

### 本地故障诊断

运行记录页支持按状态、具体事件和文本联合筛选，显示匹配条数并一键重置。点击「生成诊断报告」获取可预览的本地 JSON 快照，仅包含白名单状态、时间、计数、开关与事件类型，不含账号、密码、IP、MAC、路径或原始错误；无网络发送。复制失败时可手动复制。报告时间戳仍可能暴露使用时间，分享前自行检查；它不是校园网服务器会话记录。
