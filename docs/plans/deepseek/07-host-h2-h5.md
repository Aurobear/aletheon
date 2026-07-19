# Host Platform H2–H5

**总状态：前置条件未满足，禁止执行。**

## H2 Windows Core

解锁条件必须同时满足：W5-10 通过；CI 存在受信任 Windows Server 2022 runner；runner 可运行 Job Object、ConPTY、SCM 和 `ReadDirectoryChangesW` 集成测试。完成路径固定：`platform-windows` probe → ProcessHost/CreateProcessW+Job Object → FilesystemHost → ConPTY → SCM → contract suite。缺少任一条件时禁止创建 crate。

## H3 macOS Core

解锁条件必须同时满足：H2 全部完成；CI 存在受信任 macOS 15 runner；runner 可运行 `posix_spawn`、FSEvents、launchd 和 Keychain 集成测试。完成路径固定：`platform-macos` probe → ProcessHost → FilesystemHost/FSEvents → PTY → launchd → Keychain → contract suite。

## H4 Desktop Capability

解锁条件：H2、H3 完成；Linux X11/Wayland、Windows interactive desktop、macOS GUI 三类 runner 均登记。实现顺序固定 Linux → Windows → macOS；每个 backend 必须实现 display、input、clipboard、window enumeration、accessibility receipt。无 interactive runner 时禁止执行对应 backend。

## H5 生产运营

解锁条件：H4 完成且三 OS contract suite 连续 20 次全绿。实现固定包含签名产物、升级/回滚、崩溃转储、日志轮转、health manifest 和版本兼容矩阵。验收要求每个平台执行安装→升级→回滚→卸载，不得用 compile-only 替代。
