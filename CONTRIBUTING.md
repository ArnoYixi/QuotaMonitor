# 参与 QuotaMonitor 开发

欢迎提交问题反馈、功能建议和代码改进。先阅读 [README](README.md) 了解项目功能和构建环境。

## 本地开发

使用 Rust 1.99.0，Windows 需要 C++ Build Tools 和 Windows SDK。协议测试使用 Python 3；如未加入 PATH，可设置环境变量 `QUOTA_TEST_PYTHON` 指向 Python 可执行文件。

```powershell
cargo run --locked -- --demo
cargo fmt --check
cargo test --all-targets --locked
```

Windows 桌面回归需要可用的图形会话：

```powershell
cargo test --locked --bin QuotaMonitor desktop::tests::event_loop_unlocks_and_exits_when_rendering_is_disabled -- --ignored --exact
```

这项检查会暂停绘制，验证锁定、解锁、隐藏与退出仍能被处理。其他手工验收步骤见 [docs/ACCEPTANCE.md](docs/ACCEPTANCE.md)。

## 代码位置

| 路径 | 内容 |
| --- | --- |
| `src/desktop.rs` | 桌面事件循环和控制调度 |
| `src/ui.rs` | 界面、布局和参考图渲染 |
| `src/platform.rs` | 托盘、鼠标穿透、定位与平台集成 |
| `src/engine.rs`、`src/token.rs` | 后台状态、已有 token 和直接额度查询 |
| `src/login.rs` | 独立网页登录、PKCE、回调校验与 token 续期 |
| `src/cleanup.rs` | 升级时清理旧组件与账号缓存，保留凭据 |
| `src/migration.rs` | 旧保存位置迁移、账号保护与进程锁 |
| `src/rpc.rs` | 可选的本机 Codex 兼容通信 |
| `src/model.rs` | 额度、积分和重置卡模型 |
| `src/store.rs` | 本地设置、兼容目录与操作记录 |

托盘操作、后台结果处理和设置保存不得依赖界面绘制。修改重置流程时应保留操作记录与幂等性，不要把结果不明确的请求直接重复发送。

## 更新参考图

`docs/images/user-reference.png` 是用户提供的原始截图。其余图片从实际界面组件生成，统一使用 `arnoyix`、78%、6% 和 60325 积分，不读取真实账号。

```powershell
cargo test --locked --bin QuotaMonitor ui::layout_tests::render_readme_previews -- --ignored --exact
```

生成后检查文字、布局、百分比和主题是否正确，再提交图片。

## 发布版本

1. 更新 `Cargo.toml` 中的版本，并同步 `Cargo.lock`。
2. 添加对应的 `docs/releases/v<版本>.md`，说明变化、升级方式与限制。
3. 完成本地测试和打包自检后提交到 `main`。
4. 需要正式发布时，在 GitHub Actions 中手动运行 `Release` 工作流。它会构建 Windows 包、自检、生成校验和，然后创建对应 tag 和 GitHub Release。普通提交不会触发发布。

工作流使用仓库的 `GITHUB_TOKEN`，只在发布任务中授予写入内容的权限。已有正式 Release 不会被覆盖；发布失败时保留构建产物以便排查。发布前请确认仓库的 GitHub Actions 可正常运行。
