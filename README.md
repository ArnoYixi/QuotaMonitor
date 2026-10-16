# QuotaMonitori

原生桌面悬浮额度监视器。面向通过 OpenAI 账号登录的 **Codex 订阅额度和重置卡**，支持多个账号。Rust + egui，中文界面，不依赖 Electron、浏览器内核或 Python 运行环境。

### 0.1.3 复杂背景自适应

主界面在平坦背景上完全透明，只显示账号名、额度图形与百分比。Windows 检测到图标、照片等复杂背景时，每个账号自动增加柔和的半透明圆角衬底，强度随背景变化、最高约 58%；回到纯色背景后淡出。无边框、标题栏、工具栏或刷新时间。额度耗尽且可使用卡片时才显示「重置」。按住账号文字或进度条即可拖动；右键打开设置与账号操作，按 Esc 或「返回」收起设置。卡片数量、恢复时间与错误详情在右键后的账号详情中查看。设置中的不透明度仅影响设置面板。

靠近屏幕顶部或底部时账号横向并排，靠左右边缘时纵向排列，超出屏幕宽度时自动换行。Windows 按当前显示器工作区定位，越界后松手会拉回。额度采用鲜明的绿、黄、橙、红渐变；文字带细描边。Windows 每秒从各账号周围的透明间隙读取至多 80 个背景颜色点，同时分析明暗与颜色变化，自动切换黑白字和衬底；采样不包含自身文字、进度条或衬底，带防抖和渐变，不保存或传输屏幕图像。Linux 使用主题文字与反色描边；Wayland 的定位能力取决于合成器。

Windows 不显示任务栏窗口标签，仅保留托盘图标。托盘提供显示、隐藏、刷新、设置和退出，隐藏后仍可从托盘恢复。

## 使用

Windows x64 完整版：双击 `QuotaMonitori.exe` → 添加账号 → 在官方浏览器页面登录。可以逐个添加多个账号；登录时请确认浏览器选中了想添加的账号。支持设备码登录。

- 按住账号名或进度图形拖动窗口，默认置顶；右键打开设置和常用操作。
- 横条 / 圆环切换，绿色 → 黄色 → 橙色 → 红色连续渐变。
- 设置中选择 10 秒 / 30 秒刷新、透明度、缩放、浅色外观、锁定位置和开机启动。
- 主界面显示该账号最紧张周期的额度。右键后选择「此账号详情」查看各周期、恢复时间、点数和重置卡。
- 额度耗尽时点击「使用 1 张」，程序自动使用**最早到期的可用卡**。无到期日的卡排在最后；已过期卡不会被选中。
- 关闭窗口后留在系统托盘；托盘可恢复窗口、刷新和退出。没有可用托盘服务时，关闭即退出。
- 账号管理支持改名、排列、隐藏、暂停监测和退出登录。

### 无需账号的验收模式

```powershell
.\QuotaMonitori.exe --demo
```

提供三个模拟账号、三种额度和不同到期日的卡。可切换视图、模拟耗尽、点击使用并检查卡片变化。演示模式明确标注，使用独立数据目录，不会连接真实账号或消耗真实卡片。发布包内附 `Demo.bat`。

## 额度与重置规则

1. 数据来自官方 Codex App Server；不是估算消息条数，也不代表 OpenAI API 美元余额或所有 ChatGPT 模型额度。
2. 按服务端实际返回的周期显示，不预设一定是五小时或一周。服务端没有提供的数据显示「未知」或「—」。
3. 每次重置前重新读取账号、额度和卡片。只有真实耗尽的 Codex 配额才能触发新的操作；显示取整成 0% 不等于耗尽。
4. 在完整可用卡集合内，按到期时间、发放时间、卡片 ID 稳定排序。缺明细、明细数不匹配、到期字段缺失或类型未知时，暂不消耗。
5. 重置前将 `idempotencyKey`、卡片 ID 和账号 ID 原子写入操作日志。超时或重启后，「核对重置」复用原操作，不换卡、不生成新操作编号。
6. 未解决的操作不能移除账号；可以重新登录，但核对操作必须属于原账号。服务端不返回账号标识时暂停使用卡片。
7. 重置成功后重新查询官方数据，不自行把进度改成 100%。仅点击才使用卡片，不在额度为零时无人值守自动扣卡。
8. 网络错误显示最后已知数据；过期数据变灰。失败按指数退避，正常后恢复所选刷新频率。各账号分开轮询。

## 登录和本地数据

每个账号有独立的官方组件进程、状态目录及系统凭据条目。密码仅输入官方网页；本程序不读取浏览器 Cookie，也不会请求 API Key。默认强制使用系统凭据库；凭据库不可用时可在设置选择「仅本次会话」。此选项会在组件退出后失效，需要重新登录。

设置、上次额度和重置操作日志保存在系统应用数据目录，可在设置中打开。Windows 通常为 `%LOCALAPPDATA%\ArnoYixi\QuotaMonitori\data`，以「打开数据目录」实际位置为准。Linux 遵循 XDG 目录。不要在有待核对重置操作时删除数据目录。

程序只调用账号、登录、额度与重置方法，不创建模型会话或执行提示词。暂停账号会停止对应进程。大量同时监测的账号会增加官方组件的内存占用。

## 完整版与静态编译

Windows 完整版为单个 x64 EXE，内嵌经过 SHA-256 校验的官方 App Server 0.162.1。首次启动会将组件解压到用户数据目录；后续启动检查文件完整性。无需安装 Codex CLI、Node.js、Python、WebView2 或额外的 VC++ 运行库。

主程序使用静态 CRT / Rust 依赖；仍然调用 Windows 自带的系统 DLL 和图形驱动。静态编译不意味着完全不加载系统 DLL。内嵌官方组件使完整 EXE 约 100 MB，运行时还需解压空间；原生界面本身较小。没有证书签名，需要由使用者按自己系统的应用信任规则处理。

推荐 Windows 10 / 11 x64、支持 OpenGL 的显卡驱动。Linux 桌面源码可编译；需要 GTK3、AppIndicator、X11/Wayland 和 OpenGL。**Linux 版本不承诺全静态**；Wayland 合成器可能忽略窗口置顶或定位请求，稳定置顶优先使用 X11。Linux 托盘需要桌面的 AppIndicator 支持。

## 源码构建

固定官方组件版本和哈希在 `scripts/fetch_runtime.py` 与 `build.rs`。`Cargo.lock` 固定 Rust 依赖；CI 使用 Rust 1.99.0。

Windows 原生构建（Rust MSVC 工具链和 Visual Studio C++ Build Tools）：

```powershell
rustup toolchain install 1.99.0 --profile minimal
rustup default 1.99.0
.\scripts\build-windows.ps1
```

Linux 构建（Ubuntu / Debian）：

```bash
sudo apt-get install build-essential pkg-config libgtk-3-dev libayatana-appindicator3-dev libxdo-dev libxkbcommon-dev libssl-dev
rustup toolchain install 1.99.0 --profile minimal
rustup default 1.99.0
./scripts/build-linux.sh
```

开发版不打包组件，需在设置中指定已有 `codex-app-server` 或 `codex` 的绝对路径，或放入 PATH：

```bash
cargo run -- --demo
cargo test --all-targets --locked
```

跨平台 GUI 与系统集成在 `src/ui.rs` / `src/platform.rs`；账号工作线程在 `src/engine.rs`；协议在 `src/rpc.rs`；额度和选卡规则在 `src/model.rs`；原子持久化和重置日志在 `src/store.rs`。`tests/fake_codex.py` 是无网络故障注入服务，测试真实子进程协议。

GitHub Actions 对 Windows 和 Linux 分别测试构建，并提供构建产物。只打标签不会自动创建公开 Release。

## 验收与边界

见 [验收说明](docs/ACCEPTANCE.md)。本次开发没有使用真实用户凭据，真实多账号登录、服务端额度展示和实际扣卡需要用自己的账号验收；自动化测试覆盖协议与选卡、丢失响应、重启恢复和账号隔离。演示数据不作为真实接口验收结果。

官方接口会随服务演进，某些订阅或账号可能没有重置卡或完整明细。程序会明确显示未知状态，不用网页抓取或猜测接口填充数据。

## 来源与许可证

本项目 MIT；内置官方 Codex 组件遵循其 Apache-2.0 许可证及 NOTICE；字体遵循 SIL OFL。详见 [第三方说明](THIRD-PARTY-NOTICES.md)。

- [OpenAI App Server 文档](https://learn.chatgpt.com/docs/app-server)
- [固定版本官方组件](https://github.com/openai/codex/releases/tag/rust-v0.162.1)
- [官方凭据存储实现](https://github.com/openai/codex/blob/rust-v0.162.1/codex-rs/login/src/auth/storage.rs)

非 OpenAI 官方产品。


