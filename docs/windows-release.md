# Windows GitHub 构建与发布

所有 Windows 编译、Inno Setup 打包和安装测试均在 GitHub runner 上执行，Mac 不需要 Windows 编译工具。

## 下载预览安装包

在 GitHub Actions 打开 `Windows installer` 或 `Release`，选择成功的运行，在 Artifacts 下载 `windows-x86_64`（普通 Intel/AMD 电脑）或 `windows-aarch64`（Windows ARM）。解压 artifact 后选择 `*-setup.exe` 安装；`*.zip` 为便携版。每个架构提供 SHA256 校验。

开发分支 `release/windows-installer` 的 push 自动验证 x86_64。`Windows installer` 支持手动选择 x86_64、aarch64 或 all；工作流进入默认分支后可直接在 Actions 页面运行。合并前可通过已有的 `Release` 工作流指定此分支运行两个 Windows 架构及其它平台。

## 版本发布

`Release` 现有 `v*` 标签触发流程保留，Windows 部分复用 `windows.yml` 构建两个架构，和其它平台产物一起上传到 GitHub draft Release。人工检查安装包后再将草稿发布。不要覆盖已经发布的版本标签。

## 安装位置和数据

安装器为当前用户安装，不要求管理员权限；缺省位置为 `%LOCALAPPDATA%\Programs\RV`，可选择 `D:\RV` 等可写路径。数据默认位于实际 `rv.exe` 同目录的 `data` 子目录，快捷方式的起始目录不会改变数据库位置。显式 `RV_DATA_DIR` 仍优先覆盖。

安装器不携带用户数据库，不读取或搬迁旧 AppData。升级只覆盖程序文件；卸载保留 `data`。便携版同样使用程序旁的 `data`。用户如果删除整个程序文件夹，也会删除该目录中的数据，应先备份。

安装包及 ZIP 均包含所需 MSVC CRT DLL、图标和许可证。构建依赖从 Cargo.lock 获取，meow 依赖使用已固定的公开 Git 提交。

## 自动验证边界

工作流执行 rv-core 测试、release 编译、安装到带空格的自定义目录、同版本重装及卸载。测试核对程序文件与 `data` 位置，并检查模拟用户文件在重装和卸载后逐字节保留。GitHub runner 的这些检查不代表已验证真实桌面上的 GPU、VNC 和手机操作；需要在实际 Windows 电脑运行应用完成最终验证。
