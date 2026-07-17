# 任意工作目录、Folder Trust 与多用户工作区

## 1. 核心区分

“允许从任意目录启动”与“信任该目录中的可执行配置”是两个不同问题：

```text
cwd 可读写权限                 repo-local 配置可信度
---------------------------    --------------------------------
WorkspacePolicy 决定            FolderTrust 决定
文件/命令能在哪些根执行          hooks/MCP/plugins/LSP/.envrc 是否加载
```

Aletheon 当前 `WorkspaceSelection` 默认采用进程 cwd，可接受显式 cwd 与 add-dirs，规范化后构造 `WorkspacePolicy`（`crates/fabric/src/types/local_authority.rs:185-232`）。`WorkspacePolicy` 要求绝对 cwd，并把 cwd 和 extra roots 形成 writable roots（同文件 `:70-100`）。这说明“任意 cwd”基础已经存在；后续不应重新引入固定 `/home/.../Bear-ws` allow-root。

## 2. Grok 的 Folder Trust 决策

Grok 不用 folder trust 禁止启动，而是扫描 repo-local code-exec 配置并决定是否加载。其明确优先级为：feature off、持久信任、不可记录的宽根、无 repo config、交互询问、headless 默认不信任（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-workspace/src/folder_trust.rs:1-25`）。纯函数 `decide` 将结果限制为 `Trusted/Untrusted/Prompt`（同文件 `:35-80`）。

这对 Aletheon 的价值在于：

- 用户可以在 `/home/user`、临时目录、外部挂载或任意 git repo 启动。
- 未信任目录仍可作为普通文件工作区使用。
- 只有 repo-local 可执行扩展被禁止或询问。
- headless/daemon 无法弹窗时默认不加载不可信扩展。

## 3. 建议的 Aletheon 边界

建议把当前 `WorkspacePolicy` 保持为文件系统 authority，再新增独立的候选概念：

```text
WorkspaceLaunch
  -> WorkspaceSelection.resolve()
  -> WorkspacePolicy { cwd, writable_roots, protected_paths }
  -> WorkspaceTrustResolver.evaluate(cwd, principal, client_mode)
  -> WorkspaceTrustDecision
       Trusted { receipt }
       Restricted { blocked_sources }
       PromptRequired { findings }
```

`WorkspaceTrustDecision` 不改变 cwd 是否允许使用，只约束以下“仓库提供的执行入口”：

- repo-local hooks
- repo-local MCP server command
- repo-local plugin/skill executable
- `.envrc` 或等价环境加载器
- LSP server command
- repo-local agent definitions 中的命令型扩展

## 4. 多用户设计

Grok 的 trust store 是本机用户级文件（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-workspace/src/folder_trust.rs:3-8`）。Aletheon 是 daemon + principal 模型，不能只按 Unix home 保存一个全局布尔值。

建议 trust receipt 至少绑定：

| 字段 | 原因 |
|---|---|
| `principal_id` | Alice 的信任不能授权 Bob |
| canonical workspace identity | 防路径别名、软链接绕过 |
| repo identity / remote fingerprint（如可用） | 防同路径内容替换后继续继承信任 |
| discovered executable-config digest | 配置变化后触发重新评估 |
| granted capabilities | 可只允许 hooks，不允许 MCP/plugin |
| created/updated/expiry | 审计和撤销 |
| granting client/connection | 追踪授权来源 |

这是基于 Aletheon 已有 principal、connection、thread、turn authority 的推导；这些可信字段当前存在于 `CapabilityExecutionContext`（`crates/executive/src/service/governed_capability.rs:20-34`）。

## 5. 建议行为矩阵

| 场景 | cwd/普通文件 | repo hooks/MCP/plugins | 结果 |
|---|---:|---:|---|
| 交互式、无可执行配置 | 允许 | 无 | 直接开始 |
| 交互式、有配置、未记录 | 允许 | 暂停加载 | 请求信任 |
| headless、有配置、未记录 | 允许 | 禁止 | restricted 启动并记录事件 |
| 已信任、digest 未变 | 允许 | 按 receipt | 正常开始 |
| 已信任、digest 改变 | 允许 | 暂停加载 | 重新确认 |
| cwd 为 `/` 且隐式选择 | 保持当前拒绝 | 禁止 | 防误操作；当前规则见 `crates/fabric/src/types/local_authority.rs:206-217` |

## 6. 安全注意

- 信任不是写权限；写权限仍由 `WorkspacePolicy` 和 sandbox 管。
- 不信任不是完全拒绝工作区；应优先 restricted mode。
- 配置发现必须只读、无解释执行。
- canonicalization 后再比较路径，并防 TOCTOU；当前 Aletheon 已在 resolve 时 canonicalize（`crates/fabric/src/types/local_authority.rs:235-245`）。
- 凭证路径继续由 `ProtectedPathPolicy` 独立保护（同文件 `:120-153`）。
- 任何 trust prompt 必须带 principal/thread 归属，不能使用进程级共享 pending approval。

## 7. 验收方向

1. 从任意非 `/` 目录启动成功。
2. 不可信仓库可读写普通文件，但 repo-local executable config 不运行。
3. headless 默认 restricted，不等待不可用的交互输入。
4. 两个 principal 对同一 repo 的信任互不污染。
5. 配置 digest 改变后旧 receipt 不再自动授权。
6. trust decision、被阻止来源和授权者进入审计事件。

