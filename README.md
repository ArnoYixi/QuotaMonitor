# QuotaMonitor

轻量的桌面悬浮额度监视器，用于查看多个 Codex 账号的剩余额度、积分和重置卡。中文界面，支持横条、圆环、拖动定位与系统托盘。

[版本页面](https://github.com/ArnoYixi/QuotaMonitor/releases) · [报告问题](https://github.com/ArnoYixi/QuotaMonitor/issues) · [参与开发](CONTRIBUTING.md)

本项目由社区独立开发，与 OpenAI 无隶属或背书关系。额度和可用功能以账号实际返回的数据为准。

## 界面预览

实际使用截图：两个账号均命名为 `arnoyix`，剩余额度分别为 **78%** 和 **6%**，第二个账号显示 **60325 积分**。

![深色背景下的横向悬浮窗](docs/images/user-reference.png)

下面的参考图由程序的实际界面组件渲染，固定使用上图的数据，用于展示不同布局和外观，不表示实时额度。

| 纵向横条 | 横向圆环 |
| --- | --- |
| ![纵向横条：78%、6%、60325 积分](docs/images/bars-vertical.png) | ![横向圆环：78%、6%、60325 积分](docs/images/rings-horizontal.png) |

浅色背景：

![浅色背景横向横条：78%、6%、60325 积分](docs/images/bars-light.png)

## 功能

- **多账号监测**：每个账号独立网页登录，自动接收 token，支持改名、排序、隐藏或暂停监测；在详情页查看各额度周期和恢复时间。
- **桌面悬浮**：横条与圆环两种样式，支持缩放、置顶和拖动。靠近顶部或底部时横向排列，靠近左右边缘时纵向排列。
- **自适应外观**：Windows 下根据背景调整文字颜色；复杂背景下自动添加柔和衬底，纯色背景保持透明。
- **托盘控制**：显示 / 隐藏、锁定 / 解锁按当前状态只显示可用的一项；支持刷新、设置、切换 Codex 账号与退出。锁定后悬浮窗穿透鼠标，托盘仍可直接解锁和退出。
- **积分与重置卡**：积分向上取整，不显示小数。token 模式只查询，使用重置卡请前往官方客户端。
- **本地运行**：Rust + egui 原生程序；直接查询额度，无需 Python、Node.js 或 Codex CLI，不下载插件或模型目录。

## 下载与使用

目前尚未发布二进制 Release，可按下方“从源码构建”生成程序。发布包的使用方式如下：

1. 在 [Releases](https://github.com/ArnoYixi/QuotaMonitor/releases) 下载 `QuotaMonitor-v版本号-windows-x64.zip`。
2. 解压到固定目录，双击 `QuotaMonitor.exe`。
3. 选择添加账号，程序会打开官方登录网页。完成登录后自动导入 token 并查询额度，无需查找本机登录文件。已有账号可在详情中点击“浏览器登录”；也可在“手动导入已有 token”中粘贴 token。
4. 拖动账号名或进度图形调整位置；右键悬浮窗，选择“打开菜单”管理账号和外观。

Windows 发布包面向 Windows 10 / 11 x64，需要支持 OpenGL 的图形驱动。程序尚未进行代码签名。

### 常用操作

| 操作 | 方法 |
| --- | --- |
| 切换横条 / 圆环、刷新间隔、缩放 | 打开菜单 → 设置 |
| 查看各周期、恢复时间、积分明细 | 打开菜单 → 管理账号 → 详情 |
| 切换 Codex 登录账号 | 右键系统托盘图标 → 切换 Codex 账号 → 选择已登录账号 |
| 锁定悬浮窗 | 右键悬浮窗或托盘图标 → 锁定悬浮 |
| 解锁悬浮窗 | **右键系统托盘图标 → 解锁悬浮** |
| 隐藏 / 恢复窗口 | 使用托盘菜单；左键托盘图标可显示窗口 |
| 完全退出 | 托盘菜单 → 退出 |

锁定后，悬浮窗本身不接收鼠标操作。解锁立即生效，无需先隐藏再显示；左键托盘图标只显示窗口，不会解除锁定。关闭设置窗口会留在托盘，没有可用托盘时则退出。

### 无需登录体验

运行发布包中的 `Demo.bat`，或：

```powershell
.\QuotaMonitor.exe --demo
```

演示模式使用独立的模拟数据，不会登录账号或消耗重置卡；其动态演示数据与 README 的固定参考图不同。

## 数据与隐私

- 默认用已有 access token 直接请求 `https://chatgpt.com/backend-api/wham/usage`，不启动 Codex App Server。该接口沿用客户端返回格式，未来服务变更可能需要更新程序。缺失字段显示未知；请求失败时保留额度缓存并标明状态。
- 网页登录使用 PKCE 与本机回调，完成后完整凭据保存在各账号的 `auth.json` 中，结构与 Codex 原生文件一致，保留 `id_token`、`access_token`、`refresh_token`、`account_id` 和 `last_refresh`。凭据不进入设置文件；旧版 `token.secret` 会自动迁移为此格式。
- 不会自动查找或导入 Codex 本地登录。手动在托盘选择切换后，才将该账号的 auth.json 写入 `$CODEX_HOME/auth.json`（默认 `%USERPROFILE%\.codex\auth.json`），写入前将原文件备份为 `auth.json.quotamonitor-backup`。已运行的 Codex 需重新打开以载入切换后的登录。
- 仅对用户选中的切换账号同步目标 auth.json 的续期结果，避免两边使用不同的新旧 token。目标文件若换成其他账号，不会用原账号覆盖它；没有切换过的账号不会读取 Codex 的 auth.json。
- 浏览器仅在对应账号的登录信息成功保存后显示完成；随后额度条显示“查询中”。额度已用完时正常显示 0%，保留积分和恢复时间。
- 设置和账号直接保存在 `%LOCALAPPDATA%\QuotaMonitor\`，没有发布者名称层级和额外的 `data` 子目录。Linux 使用 `~/.local/share/quotamonitor/`；也可用 `--data-dir <目录>` 指定位置。
- 升级时自动迁移旧版 QuotaMonitor / QuotaMonitori 的账号、设置与待核对记录，保留原有登录，并删除迁空的旧目录。已有新位置账号时优先使用，不覆盖它们。已有开机启动项会更新为新程序路径。
- 0.1.8 启动时默认切换为直接查询，并清理本数据目录中的旧 `runtime`、账号插件、缓存、临时文件和日志。Windows 下会用系统凭据库中的原密钥迁移 QuotaMonitor 自己的旧登录，保存成功后继续查询，无需统一重新登录。保留账号凭据、设置及待核对的重置记录；原密钥不可用或登录已失效时才提示重新网页登录。旧版文件绑定引用会删除，原文件不受影响。被占用的旧文件下次启动继续清理。
- 背景自适应只读取窗口周围少量颜色点，不保存或上传屏幕图像。
- 重置卡属于账号资源；默认 token 查询不消耗卡片。设置中的旧版兼容方式仍可指定本机 Codex，并保留原有重置核对逻辑。

## 从源码构建

构建环境：Rust 1.99.0；Windows 还需 Visual Studio C++ Build Tools 和 Windows SDK。Python 3 仅用于构建时收集许可证。

```powershell
git clone https://github.com/ArnoYixi/QuotaMonitor.git
cd QuotaMonitor
rustup toolchain install 1.99.0 --profile minimal --component rustfmt
rustup override set 1.99.0
.\scripts\build-windows.ps1
```

输出位于 `dist/`。默认构建不下载、不嵌入 Codex 组件。

运行 `scripts/package-windows.ps1` 可进行 token 查询方式自检并生成 ZIP 和校验和，输出位于 `dist/releases/v<版本>/`。

Linux 源码构建：

```bash
sudo apt-get install libgtk-3-dev libayatana-appindicator3-dev libxdo-dev libxkbcommon-dev libssl-dev
./scripts/build-linux.sh
```

Linux 的托盘、定位和窗口行为依赖桌面环境；Wayland 尤其受合成器限制。目前打包脚本面向 Windows x64。

开发调试与测试：

```powershell
cargo run --locked -- --demo
cargo fmt --check
cargo test --all-targets --locked
```

旧版登录兼容方式可在设置中指定本机 Codex CLI 路径。更多回归步骤见 [验收说明](docs/ACCEPTANCE.md)。

## 反馈与贡献

欢迎在 [Issues](https://github.com/ArnoYixi/QuotaMonitor/issues) 反馈问题或提出建议。请附系统版本、程序版本、复现步骤和已脱敏截图；不要提交账号凭据、Token 或完整个人数据目录。开发说明见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可证

项目源码采用 [MIT License](LICENSE)。字体采用 SIL Open Font License；其他依赖详见 [第三方声明](THIRD-PARTY-NOTICES.md) 和发布包中的 `licenses/`。旧版 Codex 组件的许可证资料仍保留，但默认发布包不含该组件。
