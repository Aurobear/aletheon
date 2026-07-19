# Aletheon Host Platform 收敛计划

> **Status:** Linux contract wired; filesystem TOCTOU and native cross-OS evidence remain open
>
> **Verified:** 2026-07-20

## 1. 定位

`platform` 是 Aletheon 访问宿主操作系统能力的唯一领域 crate：

```text
Corpus / execd / Executive adapter
              |
              v
          platform
     contract + selector
       + OS backends
```

它负责文件系统、进程、PTY、服务、sandbox、desktop 与 capability probe；不负责
Agent、模型、Goal、机器人设备或权限策略。

## 2. 当前代码

单 crate 导出 contract、registry、selector 与 backend：
`crates/platform/src/lib.rs:1-38`。

```text
crates/platform/src/
  filesystem.rs process.rs pty.rs sandbox.rs service.rs
  manifest.rs path.rs receipt.rs desktop.rs
  registry.rs selector.rs
  backend/linux
  backend/windows
  backend/macos
```

Linux 真实类型已经导出：
`crates/platform/src/backend/linux/mod.rs:20-31`。

文件系统权限目前按单次操作投影，而不是由 Platform 自行决定：Executive 将
`WorkspacePolicy` 路径写入 capability 请求，Kernel 返回的 granted scope 随工具调用
authority 到达 Corpus，Corpus 再与 workspace/protected-path 规则取交集并创建
`FilesystemScope`。空 filesystem path scope 在该 adapter 上 fail closed；外部普通文件
只允许 Permit 中的精确路径只读，外部写入不开放。`file_read`、`file_write`、`apply_patch` 和 execd patch 已走此路径。`apply_patch` 的 unified diff 和 structured
patch 均由 Corpus 解析，但所有实际读取、原子替换和删除都交给 operation-scoped
Platform handle，修改和删除携带读取时的内容 hash 以拒绝 stale workspace view；不再
启动拥有 ambient filesystem authority 的系统 `patch` 进程：
`crates/executive/src/service/governed_capability.rs:516-562`、
`crates/corpus/src/tools/tools/scoped_filesystem.rs:12-77`、
`crates/corpus/src/tools/tools/apply_patch.rs:116-184`。

```text
WorkspacePolicy + requested path
              |
              v
       Kernel granted scope
              |
              v
 Corpus workspace/protected intersection
              |
              v
 platform::FilesystemScope -> native backend
```

Linux backend 先用 lexical/canonical 信息选择 admitted root，再以 scope 构造时固定的
root/read fd 执行实际 I/O。路径打开使用
`openat2(RESOLVE_BENEATH|RESOLVE_NO_MAGICLINKS)`；Deny policy 追加
`RESOLVE_NO_SYMLINKS`。create/write/remove 使用 pinned parent directory fd，避免验证后
重新从 ambient absolute path 打开；内核不支持 `openat2` 时 fail closed：
`crates/platform/src/backend/linux/filesystem_host.rs:414-490`。

## 3. 当前接线状态

Selector 已在对应编译目标选择原生 Linux、Windows 或 macOS probe，未知 OS
保持 fail closed：`crates/platform/src/selector.rs:26-59`。Linux 已通过统一入口
返回真实 OS 版本、探测时间和 capability state。共享 Linux contract suite 当前有 19
项行为测试，覆盖 scoped filesystem、process timeout/tree cleanup、PTY resize/EOF、
service typed state/error、sandbox observed strength 与 bounded receipt：
`crates/platform/tests/contract_suite.rs:1-40`。

Windows/macOS 代码只能在对应 runner 上验证；在 Linux 编译 stub 不能证明原生实现
正确。

## 4. 内部边界

```text
contract
  stable types and traits; no OS handle leaks

selector/registry
  choose exactly one backend and report capability state

backend/linux
backend/windows
backend/macos
  translate contract to native OS APIs
```

Contract 中不得出现 systemd、Win32 handle、launchd、Cocoa 或具体 sandbox 工具类型。
Backend 不得授予 capability permission，只执行已授权请求。

## 5. 实施顺序

1. ~~让 selector 在当前 OS 返回真实 backend；未知 OS fail closed；~~
2. ~~扩充 Linux contract suite，覆盖 filesystem/process/PTY/service/sandbox；~~
3. ~~删除 selector 与各 backend 内重复 stub 语义；~~
4. 在 Windows 原生 runner 验证 Job Object、ConPTY、SCM 与 filesystem；
5. 在 macOS 原生 runner 验证 process、PTY、launchd、FSEvents/TCC；
6. ~~Corpus filesystem/patch 通过 Platform contract，并删除无 caller 的重复 Host 树；~~
7. ~~Execd 依赖最小 Platform/patch contract，而不是完整 Corpus。~~

## 6. Crate 约束

保持一个 `platform` crate。只有下列事实出现时才重新讨论拆分：

- 必须隔离的重量级 SDK；
- 无法在同一 package 表达的构建工具链；
- 独立发布/部署生命周期；
- 明确安全边界和真实生产 caller。

“每个 OS 一个 crate”或“API 与 host 分层”本身不是理由。

## 7. 验收

- `platform::probe()` 返回真实当前 OS 与真实 feature state；
- contract tests 对每个原生 runner 使用同一套语义；
- unsupported 与 permission denied 可区分；
- 路径、进程、PTY 和 service 操作有 bounded receipt；
- 无 Platform -> Corpus/Executive 反向依赖；
- 不存在 `platform-api/platform-host/platform-linux/...` package。
