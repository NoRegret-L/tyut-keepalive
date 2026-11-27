# 许可证与范围

项目维护者报告 NoRegret-L 与 Code-famer-horse 双方同意对有权授权的项目自有源代码采用 Mozilla Public License 2.0（SPDX: MPL-2.0）。

- 根目录 [LICENSE](LICENSE) 为完整 MPL-2.0 许可证文本，来源为 SPDX license-list-data 的 MPL-2.0.txt（blob ee6256cdb62a765749a71aae3abea32884301cd1）；官方文本：https://www.mozilla.org/en-US/MPL/2.0/ 。
- 自有 Rust、前端 JS/HTML/CSS 和 Python/JS 开发脚本以 SPDX-License-Identifier: MPL-2.0 标记。项目自有配置及文档同样采用 MPL-2.0；不可写注释的 JSON 等文件以此目录级声明为准。
- src-tauri/vendor/ 中的第三方代码（含本地补丁）保留其原许可证、版权及来源记录，不由本项目重新授权；Cargo.lock、package-lock.json 仅为依赖元数据。
- src-tauri/gen/ 为生成的 schema，src-tauri/icons/ 与 dist/app-icon.png 等图像素材来源和授权须单独复核；本声明不对第三方素材、校名标识或商标授予额外权利，也不宣称未经核实的版权归属。
- 若分发受 MPL 覆盖的可执行程序，须按第 3.2 节告知获取对应版本源码的方式，并保持源码许可及相关声明。应指向与安装包对应的提交或正式标签。第三方许可证与分发义务另行核对（Issue #3、PR #7）。本项目采用通常的 MPL-2.0，不额外附加 Exhibit B 的不兼容声明。
- 本次仅建立自有源代码许可，不是对第三方许可证、图标或安装包整体的分发批准；不创建 Tag 或 Release，不改变更新签名或发布门禁。
