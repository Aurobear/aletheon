# Aletheon 全项目 Bug 诊断与修复计划

> **创建日期：** 2026-08-27
> **最近校准：** 2026-08-28 16:16 CST
> **代码基线：** commit `f4b22ec8`（dev 分支）；本文档当前位于未跟踪的
> `docs/plans/` 目录，不把“工作区干净”作为取证前提。
> **性质：** 活跃修复台账；工作区包含尚未提交的实现。数据库清理、提权
> 部署、测试源码和核心控制范围仍按 AGENTS.md 门禁单独授权。
> **取证方法：** 源码审读 + 活体系统取证（`~/.local/state/aletheon/`
> 状态库统计、journalctl 结构化日志聚类、`aletheon doctor`、
> `ps` RSS）+ 两个只读审计子代理（daemon/kernel 14 项、TUI 26 项）。
> **关闭规则：** 引用 `docs/design/roadmap/runtime-correctness-backlog.md`
> 的维护规则 —— 问题关闭必须通过安装态验收
> （`sudo bash scripts/aletheon.sh deploy` + digest/重启/真实请求证据），
> 开发二进制与直接 RPC 仅作诊断。

### 状态口径

| 状态 | 含义 | 能否直接进入实现 |
|---|---|---|
| `Confirmed` | 当前基线代码或可重复运行证据已证实机制 | 可以，但仍需单独批准写范围 |
| `Needs evidence` | 有现象或风险，因果/影响/复现尚不足 | 不可以，先补只读取证 |
| `Product decision` | 代码行为明确，但期望交互或预算语义未定 | 不可以，先由开发者裁决 |
| `Excluded` | 本轮复核发现原描述与代码不符 | 不进入修复批次；保留记录避免重复排查 |

“代码确认”只证明机制存在，不等于证明它造成了某个运行指标。特别是
RSS、延迟和成功率结论必须有时间序列或可重复场景，不能由单个快照推出。

---

## 0. 执行摘要

用户主诉“实际使用体验很差”。代码与活体取证共同确认了五条主要问题链；
其中运行影响仍需补证的条目已在正文显式标为 `Needs evidence`：

| # | 根因 | 一句话机制 | 直接体感 |
|---|------|-----------|---------|
| R1 | 协议事件同步每 50ms 全量重放会话历史 | 订阅 tail 轮询 → 全量 reload → 逐条 UPDATE | **会话越长 UI 越卡**，磁盘持续写 |
| R2 | 记忆提案子代理 30s 等待栅栏 vs 30s+ 的真实推理 | `CANCEL_WAIT=30s` + 错误折叠成 Internal | **记忆系统 9/9 全部失败** |
| R3 | TUI 每 200ms 重新下载全量快照 + 每帧克隆全转录 | `ReloadSnapshot` 每页触发 + per-frame clone | TUI 随历史增长变慢、发热 |
| R4 | 单 provider + 串行重试 + 300s 流空闲超时 | 首次请求后最多 4 次同源重试，可阻塞数分钟才 failover | **单个 turn 3~13 分钟** |
| R5 | 状态库只增不删 | 快照全量序列化、1153 个 space、无 DELETE | **self_field.db 9.7GB / 全目录 ~12GB** |

实测数据（2026-08-27 取证）：

- 状态目录总占用 ~12GB，其中 `self_field.db` 9.7GB（`self_snapshots`
  表 280 行，最大单行 59,106,692 bytes；`self_events` 103,863 行，原审计
  估算其数据约 57MB）。
- journal 自 2026-08-26 起：`Runtime Agent wait failed: internal runtime
  error` ×9（全部是记忆提案路径）、`conscious read failed` ×10+。
- 原审计保留的 turn 时延摘要包括 784119ms/13 轮推理、194229ms/13r、
  121989ms/8r；第三条在原正文/附录分别写成 130682/100682ms，当前保留
  journal 无法复核，已从可引用样本中排除（见 HYG-004）。
- 2026-08-26 16:42 守护进程因
  `inference provider failed: No such file or directory (os error 2)`
  连续崩溃两次进入 systemd restart-loop。
- `aletheon doctor` 显示安装版本
  `2369d752b788740e9ddbe24b38e80f160637966e-dirty-0e734d57984b0917`，
  仓库已在 `f4b22ec8`
  —— **最近多个修复（含 TUI 响应性）未部署到正在使用的二进制**。
- 守护进程 PID 2483，22:55 CST 复核时 RSS 429584KiB，已连续运行约 1d6h。

执行分七批（§8）：先建立可比较的安装态基线，再处理 P0 安全/数据保护、
P0.5 磁盘、P1 后台正确性、P2/P2.5 延迟与 TUI，最后处理 P3/P4 长期
运行和数据卫生。数据库清理、部署和测试文件均是独立授权范围，本文不
授权执行。

### 条目状态总览

| 状态 | 条目 |
|---|---|
| `Fixed (uncommitted)` | BUG-001/004/021/022/041 |
| `Implemented (uncommitted; installed-runtime pending)` | BUG-002/005/007/008/009/010/011/012/015/017/017b/018/019/024/025/026/027a-c/029/030/031/034/036/037/038/039 |
| `Confirmed` | BUG-006/027d/032 |
| `Needs evidence` | BUG-014/016/020/023/028/035 |
| `Product decision` | BUG-003、BUG-006 的保留语义、BUG-013 的实际生产路径、BUG-027d 的键义、BUG-032 的 `0` 语义 |
| `Excluded` | BUG-033、BUG-040（均仅排除原描述；新证据可重新开放） |

---

## 1. P0 — 崩溃与数据丢失

### BUG-001 TUI 高概率崩溃：中文思考流超 4KB 触发 String::drain panic

- **状态：** `Fixed (uncommitted; deterministic regression passed and deployed;
  installed provider did not emit a >4KB thinking delta for live-path coverage)`
- **位置：** `crates/interact/src/tui/streaming.rs:106-110`
- **证据：**
  ```rust
  const THINKING_VIEW_MAX: usize = 4096; // 10 行声明
  ...
  self.thinking_buf.push_str(text);
  // Bounded tail: keep only last THINKING_VIEW_MAX bytes
  if self.thinking_buf.len() > THINKING_VIEW_MAX {
      let excess = self.thinking_buf.len() - THINKING_VIEW_MAX;
      self.thinking_buf.drain(..excess);   // 109 行：raw byte offset
  }
  ```
  `excess` 是字节偏移，无 `is_char_boundary` 检查。UTF-8 中文 3 字节/字，
  `String::drain` 在偏移落在字符中间时 panic。用户使用 fcitx5 中文输入 +
  deepseek 推理模型（长思维链），思考内容跨过 4096 字节后落点约 2/3
  概率非边界 —— **中文长思考链是高概率必现路径**（主会话已亲眼复现过
  一次 TUI 中途崩溃）。
- **根因：** 按字节截断 UTF-8 `String` 前未对齐字符边界。
- **修复：** 截断前推进边界：
  ```rust
  let mut excess = self.thinking_buf.len() - THINKING_VIEW_MAX;
  while !self.thinking_buf.is_char_boundary(excess) { excess += 1; }
  self.thinking_buf.drain(..excess);
  ```
- **验收：** 经批准的回归测试用合法 UTF-8 `&str` 增量跨过 4096 字节，
  覆盖截断偏移落在 2/3/4 字节字符内部的情况；断言不 panic、结果仍是
  合法 UTF-8 且缓冲不超过 4096 字节。安装态连续运行 3 次长中文推理，
  TUI 均保持存活。

### BUG-002 提交失败不恢复输入框 —— daemon 打嗝即丢长提示词

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：**
  - 清空时机 `crates/interact/src/tui/app/key_handler.rs:833-836`
  - 会话未初始化/生命周期路径 `crates/interact/src/tui/app/lifecycle.rs:255-261`
  - Gateway 拒绝路径 `crates/interact/src/tui/app/submit.rs:1279-1285`、
    `crates/interact/src/tui/app/submit.rs:1325-1338`
  - 提交前写 history/本地用户消息：
    `crates/interact/src/tui/app/submit.rs:1252-1257`
  - 对照组（已有本地恢复写法）：
    `crates/interact/src/tui/app/submit.rs:756-762`、
    `crates/interact/src/tui/app/submit.rs:1243-1249`
- **证据：** `submit_message` 在发送前清空输入缓冲；Gateway submit 失败 /
  会话未初始化 / 回执错误路径只追加一条系统提示，不回填输入。本地错误
  路径已有 `restore` 写法，网络/协议失败路径没有。
- **根因：** "清空-提交-确认"三段式只覆盖了本地校验失败分支。
- **修复：** 将 draft 保留到收到 `Submitted` 回执；任何非成功出口（含
  Gateway 错误、超时、会话未就绪）统一恢复同一 draft。history 与本地
  `ChatRole::User` 投影只能在成功回执后提交，或必须具备同一 submission
  ID 的幂等去重，避免“恢复后重试”产生重复历史和重复气泡。
- **验收：** 手工注入 Gateway 故障（停 daemon）提交长文本，输入框完整
  回滚；恢复 daemon 后再提交成功。
- **实现证据：** `crates/interact/src/tui/app/submit.rs:1253-1265` 仅在 typed
  Gateway 返回 `Submitted` 后写 history 和本地用户投影；其他出口恢复原
  draft、cursor、literal 输入语义和 CJK 状态。`send_to_daemon` 在
  `crates/interact/src/tui/app/submit.rs:1275-1353` 将 session 未就绪、非法
  回执、transport error 和 Gateway 缺失统一返回失败。现有 `interact` lib
  suite 183/183 通过；尚未执行安装态故障注入，不能关闭本项。

### BUG-003 Esc 清空整份草稿 —— 与 fcitx5 的 Esc=取消组字冲突

- **状态：** `Product decision`
- **位置：**
  - Esc 清空：`crates/interact/src/tui/app/key_handler.rs:938-949`
  - Ctrl+C 带内容清空：`key_handler.rs:580-587`
  - ↑/↓ 历史回填覆盖草稿：`key_handler.rs:905-927`、`key_handler.rs:146-157`
- **证据：** Esc 处理无条件 wipe 输入缓冲。fcitx5 默认用 Esc 取消当前
  前导串/组字窗口——用户想撤一个拼音串，实际丢掉整段草稿。↑/↓ 历史
  recall 直接覆盖未提交草稿，无暂存。
- **根因：** 键位语义按英文 IME 环境设计，未考虑 IME 占用 Esc/Ctrl+C
  的场景；历史回填没有 draft 暂存槽。
- **待裁决：** Esc 到达 TUI 时应“关闭浮层”“取消 pending submit”还是
  “清空草稿”，以及显式清空键位是什么。未确定交互契约前不得实现。
  历史 recall 的草稿暂存可独立处理：进入历史前暂存 draft，回到历史末端
  时恢复；不要让 Esc 同时承担 IME、历史和清空三种冲突语义。
- **验收：** fcitx5 环境下组字中途按 Esc 不丢整段草稿；↑↓ 往返后草稿
  可完整找回。

### BUG-004 服务端无界 read_line —— 本地客户端可 OOM 守护进程

- **状态：** `Fixed (uncommitted, installed-runtime verified 2026-08-28)`
- **位置：** `crates/aletheon/src/host/unix_server.rs:898`
- **证据：** `reader.read_line(&mut line)` 无长度上限。客户端侧自我设限
  1MB（`crates/gateway/src/client/mod.rs:25,284`），服务端不设防。任何
  能写 daemon Unix socket 的本地进程发一行超长数据即可把守护进程
  RSS 撑爆（当前约 420MiB RSS 仅作为风险背景，不作为该漏洞的因果证据）。
- **根因：** 信任本机客户端，缺少防御性输入上界。
- **修复：** 服务端采用与客户端 `read_frame` 同型的 `fill_buf/consume`
  有界读取，在累计长度超过 `DEFAULT_MAX_FRAME_BYTES` 前停止扩容；返回
  协议已有或新增的 typed `FrameTooLarge` 连接错误并只断开当前 client。
  单纯 `take(N)` 会把超长行截成貌似完整的请求，不能作为唯一检测手段。
- **验收：** 单元测试发送 10MB 单行，断言 daemon 返回错误而非内存
  增长；正常 1MB 内请求不受影响。
- **实现证据：** `crates/aletheon/src/host/unix_server.rs:38-66` 使用
  `fill_buf/consume` 在复制下一段前检查累计长度；连接循环在
  `crates/aletheon/src/host/unix_server.rs:930-941` 使用同一 1MB 上限。
  相邻测试覆盖 10MB 单行提前拒绝和普通换行帧正常读取。
- **安装态验证：** 对官方用户 socket 发送 10MB 单行时，服务端约在
  1.06MB 发送量处关闭该连接并记录 `wire frame exceeds the configured
  limit`；daemon PID 未变化、`NRestarts=0`，随后 `aletheon doctor` 和一次
  真实 LLM 请求均成功。该项不解决大型响应快照超过客户端上限的问题。

### OPS-001（运维项）部署滞后 —— 当前 dirty candidate 已部署

- **状态：** `Resolved for deployed dirty candidate; new worktree changes pending`
- **证据：** 2026-08-28 15:00 CST，`aletheon doctor` 报安装版本
  `f4b22ec8-dirty-22ff1f315071b998` 且 healthy；`target/release/aletheon` 与
  `/usr/bin/aletheon` SHA-256 均为
  `2fabb23bc30423556170dbe6c7d6d4386267c93947f0a3fd98c8fc200830a080`。
  machine core、user daemon 和 memory-agent 均为 active、`NRestarts=0`。
- **动作：** 该部署已覆盖 BUG-001/004/021/022/041 的当前 candidate，但不
  覆盖 15:00 后继续产生的工作区修改（从 BUG-002 开始）。下一次批次部署仍
  需获得提权授权并重新执行 §9 全套验收。
- **风险控制：** 部署前用 §9 取证命令再存一份基线（表行数、DB 大小），
  部署后对比。

---

## 2. P0.5 — 磁盘失控（12GB 的构成与清理）

### BUG-005 self_field.db 9.7GB —— 每次停机写一份全量历史快照且永不删除

- **状态：** `Implemented (uncommitted; installed-runtime and cleanup pending)`
- **位置：**
  - 写入方 `crates/dasein/src/dasein/ledger.rs:187-210`：
    `save_checkpoint()` 把 **全部事件历史** `serde_json::to_string(events)`
    序列化为 `event_prefix_json` 存入 `self_snapshots`（195 行），单行
    最大 59,106,692 bytes；
  - 触发点 `crates/dasein/src/core/mod.rs:404-418`：daemon 每次
    shutdown → `save_dasein_state`；实际 checkpoint 委托位于
    `crates/dasein/src/dasein/persistence.rs:6-8` 和
    `crates/dasein/src/dasein/reducer.rs:270-275`；
  - **全库无任何 `DELETE FROM self_snapshots`**。
- **证据：** 280 行快照占据库的绝大多数空间；同期 `self_events` 为
  103,863 行，原审计估算数据约 57MB。checksum 字段（196 行）与读取逻辑
  说明 snapshot 当前只用于前缀校验，不提供增量 replay。
- **根因：** checkpoint 是全量事件前缀副本且无保留策略；启动时仍先
  全量读取/校验 `self_events`，再解析 snapshot 只做前缀一致性检查
  （`crates/dasein/src/dasein/ledger.rs:212-240`），因此 9.7GB 副本没有
  换来增量 replay。
- **修复：**
  1. 最小代码修复：每次成功写入新 snapshot 后，仅保留最新 version；
     不在本批次引入“增量快照/投影态”新协议。若未来要让 checkpoint
     加速 replay，应另立设计，定义投影 schema、checksum 和 suffix replay
     不变量；
  2. 运维清理必须单独批准：先识别并停止所有写入该 DB 的进程，备份文件
     并执行 `PRAGMA integrity_check`，事务内只保留最大 version 的一行，
     提交后执行 `VACUUM`。当前 `PRAGMA auto_vacuum=0`，
     `incremental_vacuum` 不会回收文件空间；
  3. 清理失败时不启动写入方，保留原库和备份用于回滚，禁止边运行边删。
- **验收：** 清理后 `self_field.db` < 200MB；连续启停 daemon 10 次，
    库体积稳定不增长（每次新增 ≤ 当前事件总量大小，且旧行被 prune）。
- **注意：** `VACUUM` 需要 ~9.7GB 空闲磁盘与数分钟；先确认磁盘余量。
- **实现证据：** `crates/dasein/src/dasein/ledger.rs:197-215` 在同一事务中
  写入当前 checkpoint 并删除其他 version；删除或提交失败会回滚整个事务。
  现有 `dasein_ledger_replay` 11/11 通过。尚未部署，也未触碰真实数据库；
  10 次启停和经批准的备份/VACUUM 仍是关闭条件。

### BUG-006 意识工作区无界增长 —— 1153 个 space / 8037 行投影，只插不删

- **状态：** `Confirmed`（保留语义是 `Product decision`）
- **位置：**
  - 投影写入 `crates/agora/src/broadcast/store.rs:343-365`（insert 无
    对应 delete 路径）；
  - 消费侧 per-space 懒建 coordinator
    `crates/aletheon/src/composition/conscious_workspace.rs:490-516`；
  - 延迟绑定 slot `crates/agora/src/conscious_context_slot.rs:29-37`；
  - `care_modulation_traces`（在 self_field.db）2744 行同样无界。
- **证据：** `conscious_workspace.db` 328MB、1153 个 distinct
  `AgoraSpaceId`（每个用户会话一个 space，从不回收）；
  `extension-events.db` 822MB、`events.db` 330MB、`protocol-events-v1.db`
  176MB、`sessions` 3838 行同属只增不删家族。
- **根因：** 每会话一个 space 的设计没有配套生命周期（space 何时可
  GC）。
- **待裁决：** 先定义哪些 session/space 必须永久可恢复、终态后 grace
  period、审计事件法定/产品保留期，再选择 TTL、按会话终态 GC 或容量
  上限。不能先用 LRU 删除再倒推数据契约。care trace 与事件库按各自
  恢复/审计语义制定保留策略（见 §7）。
- **验收：** 压测 500 个短会话后 space 行数与投影行数有界；磁盘稳定。

### BUG-007 协议事件库没有 WAL —— 热路径每次写都是 rollback-journal + FULL fsync

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/adapters/sqlite/src/session/protocol_event_store.rs:19-38`
  （`Connection::open` 后未设 `journal_mode=WAL` /
  `synchronous=NORMAL` / `busy_timeout`）；
  canonical store 的建库配置见
  `crates/adapters/sqlite/src/session/canonical_store.rs:132-140`；但其后续
  每操作重开连接未重新应用 `synchronous=NORMAL`，另见 BUG-030。
- **证据：** turn 事件是 50ms 轮询内的批量写入热路径（§3 BUG-008），
  无 WAL 时每次 append 都走默认 journal + FULL sync，即磁盘 IO 放大
  ×N 且与读侧互斥。
- **根因：** 新 store 抄了 `Connection::open` 却没抄 PRAGMA 块。
- **修复：** 为 protocol store 的长生命周期连接显式配置 WAL、NORMAL、
  busy timeout；不要直接复制 canonical store 当前有缺口的重开连接逻辑。
- **验收：** 安装态外部连接可直查 durable `journal_mode=wal`；
  `synchronous=1` 必须在 store 实际写连接内通过已批准的契约测试或现有
  诊断证明，因为它是 connection-local。静默订阅不再产生 BUG-008 的历史
  UPDATE 后，再比较同一会话负载下的写字节与锁等待。只观察 WAL 不足以
  关闭性能问题。
- **实现证据：** `crates/adapters/sqlite/src/session/protocol_event_store.rs:20-41`
  在建表前配置 2 秒 busy timeout、WAL 和 NORMAL；同一长生命周期连接承担
  后续读写。`adapters-sqlite` lib suite 91/91 通过；connection-local
  `synchronous=1` 和安装态写入行为仍待部署验证。

---

## 3. P1 — 后台任务系统性失败（journal 告警的真相）

### BUG-008 协议事件 50ms 全量同步 —— "会话越长越卡"的核心机制

- **状态：** `Implemented (uncommitted; installed-runtime performance evidence pending)`
- **位置（三层耦合）：**
  1. 订阅 tail 50ms 轮询：`crates/aletheon/src/host/unix_server.rs:598`
  2. 每次轮询全量 reload + 重放：
     `crates/runtime/src/session_service.rs:125`
     （`sync_canonical_protocol_events` → 全量读 canonical → 逐条写入）；
  3. 逐条 `UPDATE protocol_events SET event_json=?3 ...`：
     `crates/adapters/sqlite/src/session/protocol_event_store.rs:49-70`
     （18 行的 UPDATE 语句对每个 item 每轮执行）。
- **证据：** `sync_canonical_protocol_events` 对每个 canonical item 都传入
  `Some(item)`（`crates/runtime/src/session_service.rs:162-181`）；protocol
  store 命中 dedupe 后只要 `item.is_some()` 就无条件 UPDATE
  （`crates/adapters/sqlite/src/session/protocol_event_store.rs:49-65`）。因此
  静默会话也会每 50ms 全量读并重写全部历史，不是“比对后跳过 UPDATE”。
- **根因：** 同步算法没有"上次同步到哪"的水位（watermark），每次从
  seq 0 全量对齐。
- **修复：** 在 Session/protocol 同步权威中维护 per-session canonical
  item watermark，并与 protocol 写入同一事务推进；仅加载 watermark 之后
  的 canonical items。水位不能放在单个 subscription 内，否则多个订阅者
  会重复同步，重启也会丢失进度。空轮询只检查 durable tail 或由事件通知
  唤醒，不得重新扫描历史。
- **验收：** 构造 10k 事件的会话，订阅静默期 CPU 占用接近 0；新事件
  到达后 100ms 内可见；打开老会话不再出现可感知卡顿。
- **实现证据：** Runtime port 的 canonical watermark 定义位于
  `crates/runtime/src/session_protocol.rs:17-28,46-57`；SQLite adapter 在
  `crates/adapters/sqlite/src/session/protocol_event_store.rs:38-41,211-240`
  持久化水位，并在 event insert/update 的同一事务中推进。SessionService
  使用按 session 的短生命周期同步锁并仅加载 `sequence > watermark` 的
  canonical items，见 `crates/runtime/src/session_service.rs:153-170`。
  `runtime` 209/209、`adapters-sqlite` 91/91 通过；10k 静默/新事件性能验收
  和安装态 DB 写入对比仍待执行。

### BUG-009 记忆提案 9/9 失败 —— 30 秒栅栏 + 错误折叠 + 无 JSON 修复，三重叠加

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **失败链（全部已验证）：**
  1. `crates/mnemosyne/src/memory_maintenance.rs` 经 AgentControl 起子代理
     调 LLM 产出语义记忆提案；
  2. `crates/aletheon/src/composition/agent_control/runtime_bridge.rs:182-211`
     `wait_admitted` → `wait_local(CANCEL_WAIT)`，而
     `crates/aletheon/src/composition/agent_control/mod.rs:84`
     **`CANCEL_WAIT = Duration::from_secs(30)`**（1280 行传入）；
  3. `wait_for_terminal`（`mod.rs:1191-1222`）30s 未到终态 → Timeout
     错误；
  4. `runtime_bridge.rs:196` `.map_err(|_| RuntimeError::Internal)` 把
     真实原因（超时）抹掉 → journal 只剩
     `Runtime Agent wait failed: internal runtime error`（×9）。
- **另两种低频形态：**
  - LLM 输出 schema 违约无修复重试：
    `crates/mnemosyne/src/memory_maintenance.rs:195`
    `serde_json::from_str(output.trim())?` 一次失败即放弃（journal
    "invalid type: map, expected a string"）；
  - 输出被截断：`EOF at column 1098`。
- **根因：** 内部子代理复用了为交互式取消设计的 30s 常量；错误映射
  一律折叠 Internal；解析无防御。
- **修复（三件套）：**
  1. `wait_admitted` 不再复用取消专用的 `CANCEL_WAIT`；将调用侧已有的
     `run_deadline_ms`（默认 120s，
     `crates/mnemosyne/src/memory_policy.rs:26-55`）作为同一端到端截止时间
     透传，排队、推理和 wait 共同消耗预算，不能叠加多个独立超时；
  2. `wait_for_terminal` 错误透传 kind/message，`runtime_bridge.rs`
     禁止 `|_| RuntimeError::Internal` 一刀切；
  3. 优先使用 provider 支持的结构化输出；否则 parse 失败只允许一次有
     独立剩余预算的修复请求。禁止“宽松提取首个 JSON 对象”掩盖 schema
     违约；失败记录截断、脱敏后的输出摘要和明确 parse kind。
- **验收：** 至少 20 个实际 eligible intake，分别统计 admitted、排队
  timeout、provider attempt/retry、parse failure 和成功 proposal；成功率
  ≥80%，且任何失败都保留真实 typed 原因。仅“连续 3 天没日志”而没有
  intake 分母，不算通过。
- **实现证据：** Agent Runtime 的 wait port 接受调用方剩余预算并保留 typed
  `AgentControlError`（`crates/runtime/src/agent_writer.rs:114-124,818-825`、
  `crates/aletheon/src/composition/agent_control/runtime_bridge.rs:193-198`）；维护任务
  使用一个绝对截止时间覆盖初次生成和最多一次 JSON 修复，解析错误只记录有界、
  脱敏摘要。`runtime` 209/209、`mnemosyne` 194/194、`memory_maintenance` 10/10
  通过；20 个安装态 eligible intake 仍是关闭条件。

### BUG-010 conscious read failed ×10 —— 会话首轮读取必然失败

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：**
  - 读侧 `crates/dasein/src/core/mod.rs:431-441`：
    `effective_care_score` 用
    `AgoraSpaceId(ctx.session_id.clone())` 查询，失败即 warn 降级
    （439 行）；
  - 根因 `crates/aletheon/src/composition/conscious_workspace.rs:636`
    （674 行同型）：
    ```rust
    .context("conscious workspace has not observed a turn")?;
    ```
    per-space coordinator 只在该 space 观测到 turn 后才创建
    （`conscious_workspace.rs:490-516` 懒建）。dasein 的 care 读取发生
    在 turn 之前 → 每个 **新会话的第一次读取必然报错**；
  - 次因 `crates/agora/src/conscious_context_slot.rs:29-37` 延迟绑定
    slot 的 "not yet bound" 同样被当错误上抛。
- **证据：** 1153 个 distinct space / 8037 行投影说明这是按会话隔离的
  常态路径，不是异常；journal 自 08-26 起 10+ 次 warn。
- **根因：** "尚未观测" 是正常生命周期状态，被建模成错误。
- **修复：** 将“尚未观测”建模为显式可空结果或独立 lifecycle 状态；
  dasein 对该状态走 baseline 且不告警。真正的 DB、checksum、schema 和
  port 未绑定错误仍必须保留为错误，不能把所有 `Err` 静默吞掉。
- **验收：** 新会话首轮 turn 的 journal 无 conscious read 告警；care
  score 行为与现状一致（首轮本就 fallback baseline）。
- **实现证据：** `LatestConsciousContextPort` 的契约已明确要求 absent
  workspace 返回合法的 `latest_broadcast=None` 投影而非错误
  （`crates/contracts/src/types/conscious_arbitration.rs:23-38`）。registry 现在
  对尚未创建 coordinator 的 space 读取当前 self view，并构造、校验该空投影
  （`crates/aletheon/src/composition/conscious_workspace.rs:625-648`）；已有
  coordinator 的路径和真正的 Dasein/校验错误仍原样传播。现有
  `composition::conscious_workspace::diagnostic_tests` 5/5 通过；新会话安装态
  journal 验收仍待部署执行。

### BUG-011 provider 错误细节被抹除 —— 401/403/404/422 全都叫"provider_rejected_request"

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/adapters/inference/src/providers/provider.rs:72-82`
  （HTTP 状态码折叠为统一 terminal 错误，不带 status/body）；
  `crates/adapters/inference/src/factory.rs:218-223`（缺失 API key
  静默变成空字符串）。
- **证据：** 配置错误（key 错、模型名错、请求 schema 错）在日志里
  无法区分，排障只能猜。
- **修复：** 错误类型携带 provider identity、HTTP status 和有界、脱敏的
  body 摘要；API key、Authorization header 和响应中的敏感字段不得进入
  日志。provider 定义需显式声明是否需要凭证；只有“要求凭证但解析为空”
  才是配置错误，本地免鉴权 provider 不能被一刀切拒绝。
- **验收：** 人为配错 key → journal 出现 `401` 字样与 provider 名；
  配错模型名 → `404` + 模型名。
- **实现证据：** provider 定义新增 fail-closed 的 `requires_credentials`，本地免鉴权
  provider 必须显式 opt-out（`crates/cognit/src/config/mod.rs:608-643`）；HTTP 失败
  保留 provider identity/status 和有界脱敏 body 摘要，空凭证在 factory 阶段返回
  可行动配置错误。`adapters-inference` 相关 suite 63/63、`cognit` 386/386 通过；
  安装态 401/404 注入仍待执行。

### BUG-012 daemon 启动崩溃于 ENOENT 裸错误 —— 缺哪个依赖说不清

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **证据：** 2026-08-26 16:42 两次崩溃，日志仅
  `Error: inference provider failed: No such file or directory (os error 2)`。
  同一启动时间线显示 user daemon 首次在 16:42:03 启动并立刻失败；machine
  core 到 16:42:08 才开始并进入 active，daemon 在同一秒的第二次尝试仍失败，
  16:42:13 第三次重启才成功解析 provider。当前 user unit 仅
  `After=network.target`，没有 machine core readiness 依赖；machine core unit
  则独立挂在 `multi-user.target`。用户 `daemon.env` 当时存在且自
  2026-08-03 起未变，因此不符合“前两次缺失、第三次恢复”的时间形态。
- **代码链：** user launcher 固定连接 `/run/aletheon/core.sock`
  （`crates/aletheon/src/host/launcher.rs:88-93`）；bootstrap 立即调用 provider
  capabilities（`crates/aletheon/src/composition/daemon_bootstrap/request.rs:61-70`、
  `crates/cognit/src/ports/inference.rs:157-163`），而 core RPC client 直接把
  `UnixStream::connect` 的 IO 错误无路径上下文地折叠进 inference error
  （`crates/adapters/inference/src/core_rpc/client.rs:61-68`）。因此缺失对象已定位
  为尚未创建的 machine-core Unix socket，而非 provider 子进程或凭证文件。
- **修复：** daemon bootstrap 对 machine core readiness 做有界等待，仅对
  socket 尚未出现或连接尚未接受的启动竞态重试；权限、协议、provider 和
  配置错误必须立即失败。所有连接错误补充 core socket 路径与操作上下文，
  但不得暴露凭证。不能依赖 systemd 的偶然启动顺序或无限 restart-loop。
- **关闭前复现：** 对已确认的同类资源制造 ENOENT，journal 必须明确指出
  machine core/socket 路径；延迟 core 启动时 daemon 在有界等待内恢复且不
  增加 restart counter，core 永不就绪时以 typed readiness timeout 退出。
- **实现证据：** Core RPC client 对 ENOENT/ECONNREFUSED 做单 deadline 有界等待，
  其他错误立即失败，并在 typed error 中携带 socket 路径
  （`crates/adapters/inference/src/core_rpc/client.rs:85-123`）；launcher 在 bootstrap
  前调用 readiness（`crates/aletheon/src/host/launcher.rs:95-96`）。延迟 socket、
  永久缺失和错误分类测试通过；systemd restart counter 验收待部署。

---

## 4. P2 — 延迟与吞吐（为什么一个回答要几分钟）

### BUG-013 同一 provider 串行重试 4 次，最长阻塞数分钟才 failover

- **状态：** `Product adjudication (plan/code mismatch)`
- **位置：** `crates/cognit/src/adapters/inference/scheduler.rs:102-112,320-351`
- **证据：** `max_retries=4` 表示首次请求后最多 4 次额外重试，即同一
  provider 最多 5 次请求。无 `Retry-After` 时四次退避约为 2.5~5s、
  5~10s、10~20s、15~30s；若每次收到 60s `Retry-After`，代码还会增加
  最多 25% jitter，四次睡眠合计可达约 240~300s，期间不考虑切换。
- **修复：** 为整个 turn 和每个 provider candidate 使用显式时间预算；
  provider sleep、请求耗时和共享 admission wait 都计入预算，耗尽后切换
  已配置且 capability 匹配的候选，否则快速返回可行动错误。具体默认值
  需依据 provider 配额和长推理基准确定，不在计划里硬编码 45s。
- **验收：** 模拟 provider 持续 429：turn 在预算内切换/失败并明确告知
  用户，不再无限等待。
- **重新取证：** 当前全仓没有生产 `LlmScheduler::new` 或
  `LlmScheduler::from_providers` 装配点；仅
  `crates/cognit/src/application/event_handlers/tool_observer.rs:168,249` 调用其
  `complete`，另有测试构造。主 turn provider 由
  `crates/cognit/src/composition/inference_factory.rs:80-125` 的 canonical factory
  构建并由 harness 直接消费。原计划把 `scheduler.rs:281-365` 描述为主 turn
  数分钟延迟路径，与当前代码调用图不一致；必须由开发者裁决“排除旁路
  scheduler”还是“另立主 turn 端到端预算设计”，本轮不得把旁路修改冒充修复。

### BUG-014 并发许可=2 且流式全程占用 —— 跨会话排队最长 120s

- **状态：** `Needs evidence`
- **位置：** `crates/cognit/src/config/mod.rs:658-661`、
  `crates/adapters/inference/src/factory.rs:186`
  （`max_concurrent_requests=2` + `queue_timeout` 120s）
- **已证实代码：** 默认并发为 2、排队上限 120s
  （`crates/cognit/src/config/mod.rs:655-662`），permit 在整个 stream 生命周期
  持有（`crates/adapters/inference/src/factory.rs:171-191`）。
- **缺口：** 尚无同一有效 provider/model 下“两个长流 + 前台 + 记忆任务”
  的请求级时间线，不能只凭配置断言 BUG-009 的排队占比。
- **禁止方案：** 首字节后释放 permit 会突破 provider 实际并发边界，不得
  采用。若取证确认饥饿，应在同一机器/provider 治理边界内实现有优先级但
  防饥饿的排队，前后台仍共享真实并发和 cooldown。
- **验收：** 并发场景分别记录 foreground/background admission wait、
  provider attempts/retries 和 terminal；证明前台有界等待且后台最终获得
  服务，不以“第二个会话 <10s”这种未绑定负载的指标代替。

### BUG-015 工具参数在协议终态仍非法时静默丢失

- **状态：** `Implemented (uncommitted; installed-runtime pending)`（原“提前执行”描述已排除）
- **位置：**
  - OpenAI 仅在 `[DONE]`/EOF 完成：
    `crates/adapters/inference/src/providers/openai_provider.rs:705-719`、
    `crates/adapters/inference/src/providers/openai_provider.rs:831-850`；
  - OpenAI 终态解析失败返回 `None`：
    `crates/adapters/inference/src/providers/openai_provider.rs:906-919`；
  - Anthropic 仅在 `content_block_stop` 完成，但非法 JSON 同样返回 `None`：
    `crates/adapters/inference/src/providers/anthropic.rs:565-570`、
    `crates/adapters/inference/src/providers/anthropic.rs:701-714`。
- **证据校准：** 当前代码不会仅因中间增量暂时成为 `{}`/`null` 就提前
  执行；真实缺陷是协议已经给出 tool block/stream 终态后，非法参数没有
  转成显式失败，最终可能只剩 `Done(ToolUse)` 而没有对应 tool completion。
- **修复：** 协议终态时逐个结算所有 active tool call；合法 JSON emit
  completion，非法 JSON emit typed malformed-tool-arguments error，包含
  tool id/name 和有界摘要。不得把非法原文静默 drop，也不得执行半成品。
- **验收：** 构造流式分片使参数在 `{` 后被切开的用例，工具最终拿到
  完整参数；无效参数在终态报错而非消失。
- **实现证据：** OpenAI/Anthropic active tool state 在 block/stream 终态逐项
  `take_settled`；合法分片完成，非法 JSON 返回
  `MalformedToolArgumentsError`，仅含有界 id/name、parse kind、行列和字节数
  （`crates/contracts/src/types/llm_types.rs:112-150`、
  `crates/adapters/inference/src/providers/openai_provider.rs:912-927`、
  `crates/adapters/inference/src/providers/anthropic.rs:734-752`）。确定性分片和
  非泄漏测试通过；安装态 model-controlled argument 路径仍需连续 3 次验证。

### BUG-016 流空闲超时 300s —— 卡死的流表现为"挂 5 分钟"

- **状态：** `Needs evidence`
- **位置：** `crates/cognit/src/config/mod.rs:453`
  （`stream_idle_timeout_ms = 300_000`）
- **证据边界：** 当前注释明确说明 90s 曾误杀长 reasoning
  （`crates/cognit/src/config/mod.rs:447-453`），因此不能仅因 300s 体感差就
  直接降到 60~120s。
- **下一步：** 分别测量正常长 reasoning 的首字节/块间静默分布和真实
  卡流案例；再按 provider/model 配置 idle budget。无论默认值如何，超时
  必须向 UI 投影明确的 provider、阶段和 elapsed，不得静默重试。
- **验收：** 人为挂起 provider 流，UI 在超时值内收到明确错误。

### BUG-017 Gateway 客户端无读超时 —— daemon 卡死则客户端永久挂起

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/gateway/src/client/mod.rs:298-341`（`request()` 无
  read timeout）
- **修复：** 给 request class 配置有界 deadline，并返回 typed timeout；
  session read、控制命令和可能长时运行的操作不能共享一个武断的 30s。
  超时后的 request ID 必须终结或隔离，迟到响应不得被误配给下一请求。
- **验收：** sigstop daemon 后客户端在该 request class 的配置 deadline 内
  返回 typed timeout 而非永久阻塞；恢复后迟到响应不会污染下一请求。
- **实现证据：** `RequestDeadlines` 为 control/query/maintenance 三类请求提供
  可覆盖的 30s/30s/300s 默认期限，并把 compact、transaction review、extension
  和 workspace restore 归入 maintenance，而不是强制所有操作共用 30s
  （`crates/gateway/src/client/mod.rs:27-60`）。整个写入+读取关联过程受 deadline
  约束；超时返回既有 typed `ProtocolError::Timeout`，并在显式 reconnect 前拒绝
  复用已污染的流（`crates/gateway/src/client/mod.rs:346-369,471-505`），因此迟到
  response 不能被下一请求消费。`gateway` lib suite 9/9 通过；SIGSTOP 与安装态
  重连验收仍待部署执行。

### BUG-017b 订阅 tail 出错静默死亡 —— UI 冻结无任何报错

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/aletheon/src/host/unix_server.rs:604-607`
- **证据：** tail 轮询出错仅 warn 后 `return`，客户端不知道订阅已断
  → UI 事件流冻结（配合 §5 BUG-021 构成"假死"）。
- **修复：** 出错时向客户端推送 typed subscription terminal（含最后确认
  cursor 和可重试性）；客户端据此重连并按 cursor 续传（联动 BUG-018）。
- **验收：** kill 订阅路径 → TUI 收到断线提示并自动重连。
- **实现证据：** versioned subscription tail 失败会发送带最后 cursor、stable code
  和 retryable 的 `SubscriptionTerminal`，消息限制为 256 字符
  （`crates/contracts/src/protocol/client.rs:1519-1526`、
  `crates/aletheon/src/host/unix_server.rs:636-654,677-685`）。ACP/TUI reducer 均保留
  cursor 和 retryability；安装态故障注入待部署。

---

## 5. P2.5 — TUI 假死与渲染（子代理深审 26 项中的关键项）

### BUG-018 事件流断开直接 break，无提示无重连

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/interact/src/tui/app/lifecycle.rs:405`
- **证据：** `ConnectionClosed` 只跳出本轮最多 32 个事件的 poll 循环；
  不提示、不清状态，也不在该路径触发 reconnect，后续循环会继续遇到已
  关闭 transport，表现为输出冻结但转圈仍在。
- **修复：** 将连接状态作为显式 UI state；断开后提示“连接断开，重连中”，
  在后台按有界退避重连，成功后从最后 authoritative cursor 续传。键盘和
  渲染循环不能等待 socket。
- **实现证据：** `PresentationConnectionState` 明确区分 connected/recovering/
  disconnected，transport 被移入 bounded background reconnect task
  （`crates/interact/src/tui/controller.rs:14-104`）；事件 poll timeout 保持普通 idle，
  `ConnectionClosed` 才进入恢复并从现有 projection cursor 续传
  （`crates/interact/src/tui/app/lifecycle.rs:447-530`）。

### BUG-019 投影失败停轮询不清标志 —— 永久转圈

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/interact/src/tui/app/lifecycle.rs:511-518`
- **修复：** projection transport 失败进入显式 disconnected/recovering
  状态并停止 spinner；只有 authoritative terminal 才能把 turn 标为成功或
  失败。watchdog 可以提示“进度未知”并触发重连，但不得按本地超时伪造
  terminal。
- **实现证据：** projection transport failure 停止 `streaming`/spinner 但保留
  `turn_active`，只在 durable projection terminal 到达后结算
  （`crates/interact/src/tui/app/lifecycle.rs:489-505,603-623`、
  `crates/interact/src/tui/response.rs:639-649`）。`interact` 183/183 通过；断线
  frame、重连和 cursor 连续性待安装态验证。

### BUG-020 表格 holdback 的失败态兜底待验证

- **状态：** `Needs evidence`
- **位置：** `crates/interact/src/tui/streaming.rs:233-257` +
  `crates/interact/src/tui/response.rs:229-247` +
  `crates/interact/src/tui/reducer.rs:742-749`
- **证据校准：** live tail 的表格确实会 holdback 到 commit 或出现表格后的
  非 pipe 文本；但 typed Completed item 会直接提交 durable assistant，
  因此“只丢 TurnDone 就永久不可见”不成立。需要复现同时缺少 live terminal
  和 durable projection 的场景，确认表格是否仍被扣留。
- **候选修复：** authoritative terminal 到达时强制 flush；连接进入明确
  failed/recovering 状态时，允许 presentation-only 的有界 holdback 超时
  展示已有文本，但不得借此宣告 turn 成功。

### BUG-021 每收一页事件强制 ReloadSnapshot —— 每 200ms 全量重下快照

- **状态：** `Fixed (uncommitted, installed-runtime verified with BUG-041 on 2026-08-28)`
- **位置：** `crates/interact/src/tui/reducer.rs:643-718`（每 event
  page → `ReloadSnapshot`）
- **证据：** 活动期间每 ~200ms 轮询周期重新下载整个会话快照 +
  `items.clear()` 重建。这是"TUI 随使用变慢"的第二机制（R3），与
  服务端 BUG-008 叠加。
- **修复：** 事件页正常合并入现有 items；仅结构性变化（session 切换、
  gap 检测）才触发全量 reload。
- **实现证据：** `crates/interact/src/tui/reducer.rs:754-763` 在页通过 schema、
  session、cursor 和 ordered-tail 校验后合并事件并返回 `SubscribeAfter`；只有
  结构性校验失败分支返回 `ReloadSnapshot`。BUG-041 的 153-item 安装态恢复
  同时覆盖了该增量页路径；确定性的 reducer 分页回归仍待测试源码授权。

### BUG-022 每帧克隆整个转录并重测宽度 —— O(transcript)/帧

- **状态：** `Fixed (uncommitted, installed-runtime verified 2026-08-27)`
- **位置：** `crates/interact/src/tui/task_console.rs:188-248`
- **证据：** durable markdown 解析已有缓存，但每帧仍克隆每个缓存的
  `Vec<Line>`、重建总行向量并对全部行调用 `line.width()`。实际帧间隔还
  包含 poll/RPC/渲染耗时，不能笼统写成恒定 50ms；复杂度仍是
  O(total rendered lines)/frame。
- **修复：** 行缓存带 measured_width 失效标记（width 变化才重测）；
  增量只动末尾可见窗口。
- **实现证据：** `crates/interact/src/tui/state.rs:115-135` 保存按终端宽度
  组成的完整换行布局；`crates/interact/src/tui/task_console.rs:200-267` 仅在
  内容/宽度变化时重建，纯滚动只复制可见窗口；
  `crates/interact/src/tui/task_console.rs:875-900` 覆盖 150-item 会话滚动且
  断言布局缓存未重建。
- **安装态验证：** `/usr/bin/aletheon` 通过官方用户 socket 恢复 30-message
  真实会话；单次鼠标滚轮事件 29ms 内改变 frame，20 次连续上滚 585ms 内
  稳定，反向滚动 627ms 内恢复初始尾部。153-item 会话另触发
  `wire frame exceeds the configured limit`，属于独立的大快照帧上限问题，
  不作为本项滚动性能结论。

### BUG-023 RPC 类按键 inline await —— 残余冻结待安装态复测

- **状态：** `Needs evidence`（代码机制确认，安装态残余影响待复测）
- **位置：** `crates/interact/src/tui/app/lifecycle.rs:278`
  （`handle_key` 内 await Gateway RPC：取消/审批/mode/提交）
- **证据：** daemon 慢时每次 Ctrl+C/审批点击都冻结输入与渲染。
  `handle_key(...).await` 仍位于事件循环；commit `c9f30522` 及当前 dirty
  candidate 已部署，因此下一步是直接量化残余冻结，不再等待 OPS-001。
- **候选修复：** 若复测仍失败，将 RPC 投递到后台任务并用 reducer 回执
  更新状态；键处理不直接等待网络。未复测前不重复实现已有修复。

### BUG-024 状态栏手绘格子忽略宽字符 —— 中文/emoji 乱码重叠

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/interact/src/tui/status.rs:239-252`、
  `crates/interact/src/tui/status.rs:346-359`
- **证据：** `x += 1` 逐字符填充不看显示宽度，中文与 emoji 图标在
  状态栏重叠乱码 —— 中文用户日常可见。
- **修复：** 改用 ratatui 原生 widget/`Line` 拼接（其内部处理宽字符），
  删除手绘 cell 循环。
- **实现证据：** AppState 与 legacy 两条状态栏路径均改由 ratatui
  `Paragraph(Line)` 渲染，删除逐 char 的 cell 写入；legacy 右对齐也改用
  `Line::width()` 而非 UTF-8 byte length
  （`crates/interact/src/tui/status.rs:229-231,305-317`）。现有 status 测试 3/3
  通过；中文/emoji 安装态 frame 验收待部署。

### BUG-025 输入框单行无横向滚动 —— 超宽输入盲打

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/interact/src/tui/render/renderable.rs:191-232`
  （输入行 1 行高、无横向滚动/光标跟随；`input_line.rs` 的
  `render_input` 是死代码）
- **证据：** 输入超过终端宽度后看不见自己在打什么；多行输入
  （Alt+Enter）只显示第一行。
- **修复：** 输入区按光标位置横向滚动；多行时显示末 N 行 + 行数指示。
- **实现证据：** 输入布局扩为 separator + 最多三行 viewport；当前逻辑行始终
  可见并按 ratatui 显示列横向滚动，多行显示 `current/total`
  （`crates/interact/src/tui/render/renderable.rs:190-292`、
  `crates/interact/src/tui/render/draw.rs:95-104`）。真实宽字符 frame 待安装态验证。

### BUG-026 计时器使用合成 tick + 底栏文案误导

- **状态：** `Implemented (uncommitted; installed-runtime pending)`（“快 20%”这一固定比例已撤回）
- **位置：** `crates/interact/src/tui/status.rs:49-55`（每 tick 固定 +0.06s）、
  `crates/interact/src/tui/status.rs:222-228`（写 “Ctrl+C quit”，但单次
  Ctrl+C 实际是取消活动 turn 或清空输入）
- **证据校准：** streaming poll 常用 50ms，但循环还包含其他工作，误差
  不是稳定 20%；问题是显示时间没有来自单调时钟。
- **修复：** 计时用 `Instant::elapsed`；文案与实际键位语义对齐。
- **实现证据：** 计时改由注入的 monotonic `Clock` 起点计算，不再累计合成 tick
  （`crates/interact/src/tui/status.rs:51-70`、
  `crates/interact/src/tui/app/lifecycle.rs:386-389`）；底栏明确单次 Ctrl+C 是
  cancel/clear、双击才退出。确定性 275ms 时钟测试通过。

### BUG-027a 无法解码的事件仅写 tracing，TUI 无可见提示

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/interact/src/tui/response.rs:12-32`
- **修复：** 保留 tracing 详情，并向 TUI 投影一条有界、去重的协议错误；
  若能提取 cursor/seq 则显示，否则不得虚构序号。高频坏事件必须限流。
- **实现证据：** 双重 decode 都失败时仍写详细 tracing，但同时投影固定长度的
  可见协议错误；模型用单调时钟把同类通知限制为每 5 秒最多一次，消息不虚构
  cursor/seq（`crates/interact/src/tui/response.rs:12-40`、
  `crates/interact/src/tui/model.rs:64-69`）。`interact` lib suite 183/183 通过；
  安装态坏事件注入和限流观察待部署。

### BUG-027b `/copy` 绕过 ratatui 且吞掉 stdout 错误

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/interact/src/tui/app/submit.rs:813-838`
- **修复：** 通过终端能力层发送 OSC52 并传播失败到系统提示；不要在
  ratatui 绘制期间无协调地直写 stdout，也不能在失败时仍显示“已复制”。
- **实现证据：** OSC52 输出集中到终端能力层，以一次 stdout lock 完成写入和
  flush 并返回原始 IO error（`crates/interact/src/tui/term_compat.rs:219-227`）。
  `/copy` 只有成功后才显示已发送；失败会写入可见 error，不再 `.ok()` 吞错
  （`crates/interact/src/tui/app/submit.rs:812-840`）。`interact` lib suite
  183/183 通过；支持/拒绝 OSC52 的真实终端验收待部署。

### BUG-027c `/mode` 未知值静默回落 Default

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/interact/src/tui/app/submit.rs:965-985`
- **修复：** 未知 mode 返回错误和可用值，不改变当前 mode。
- **实现证据：** `/mode` 现在只接受 `default/plan/auto/sandbox`；未知值在本地
  显示可用列表并在发送 RPC 前返回，因此不会改变当前 mode
  （`crates/interact/src/tui/app/submit.rs:966-996`）。`interact` lib suite
  183/183 通过；安装态交互验收待部署。

### BUG-027d 行尾反斜杠 + Enter 删除反斜杠

- **状态：** `Confirmed`（期望键义需随 BUG-003 一并确认）
- **位置：** `crates/interact/src/tui/app/key_handler.rs:812-819`
- **修复：** 明确 continuation 契约；若保留该能力，应只在显式模式生效
  或在帮助中说明，不能让普通路径/命令中的末尾反斜杠无提示消失。

### TUI 已排除项（避免重复排查）

- `chat.rs` ChatWidget 为 test-only，生产渲染走 `task_console.rs` +
  `markdown.rs`；
- resize 处理正常（宽度变化使缓存失效）；
- `deduplicate_consecutive_text` 为 test-only。
- **BUG-033 原描述已排除：** `crates/interact/src/tui/reducer.rs:194-215`
  的 guard 检查的是 durable assistant 是否已经存在，不是“overlay 未就绪”；
  尚无证据证明它丢首 delta。
- **BUG-040 原描述已排除：** Up/PageUp 会增加
  `conversation_scroll`（`crates/interact/src/tui/app/key_handler.rs:904-935`），
  渲染会应用该值（`crates/interact/src/tui/task_console.rs:250-254`），且
  delta 路径没有把它重置为 0；当前代码支持流式期间回看。

---

## 6. P3 — 后台/长时运行的内存与正确性

### BUG-028 turn_writer 无界保留历史状态 —— 长时内存风险待量化

- **状态：** `Needs evidence`（容器无淘汰已确认，RSS 因果未确认）
- **位置：** `crates/runtime/src/turn_writer.rs:23,28,31`
  （`terminals`/`started`/`emitted` 无淘汰）；恢复逻辑先把 replay 输入归约到
  本地 `started`/`settled`，随后把所有已启动 turn 写回 writer maps
  （`crates/runtime/src/turn_writer.rs:338-400`）。生产挂接点为
  `crates/aletheon/src/composition/daemon_bootstrap/services.rs:827-860`。
- **证据边界：** 单次 RSS 约 420MB 只能作为基线，不能证明这些容器是
  主因，也不能证明“随 turn 单调增长”。
- **下一步取证：** 在同一进程记录 turn 数、三个容器长度（优先 debugger/
  现有观测，不在热循环加永久日志）和 RSS 时间序列；区分 allocator
  high-water、render/session cache 与 writer retention。
- **候选修复：** 因 terminal fence 依赖历史状态，不能直接保留“最近
  1000 条”。先定义 durable terminal 查询/compaction 契约，再让内存只保留
  active turn 和有界幂等窗口。
- **验收：** 7 天 soak 同时报告 turn 数、容器长度和 RSS；重启恢复耗时
  与 event 数的关系有基准。没有因果数据不得关闭或宣称已修复泄漏。

### BUG-029 interrupted 集合永不清理 —— 二次中断永久失效

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/runtime/src/session_service.rs:706-723`
- **证据：** `interrupted.insert`（723 行）后无任何 remove；该会话后续
  interrupt 请求在 707 行 `contains` 检查处直接返回 AlreadyTerminal。
- **修复：** interrupted 幂等键应绑定 active turn identity，而不是永久
  session ID；active turn 替换后自然允许下一轮 interrupt。若兼容层拿不到
  turn ID，则在确认 ActiveTurnRegistry 已切换后清除旧标记，不能仅按任意
  “新请求”盲删。
- **验收：** 同一会话连续两轮"发起-中断"均成功。
- **实现证据：** interrupt 幂等键改为 `session -> Runtime canonical TurnId`，相同
  active turn 重复调用仍返回 AlreadyTerminal，registry 切换到新 turn 后可再次
  interrupt（`crates/runtime/src/session_service.rs:51-55,785-805`）。相邻两轮
  回归测试位于同文件 `:967-1001`；focused runtime 8/8 通过。

### BUG-030 canonical store 无 LIMIT 全量加载 + 每操作重开连接

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/adapters/sqlite/src/session/canonical_store.rs:476-488`
  （`load_items` 无 LIMIT）、
  `crates/adapters/sqlite/src/session/canonical_store.rs:79-86`（每操作新建
  connection，只设置 busy timeout/foreign keys）。建库连接设置 WAL/NORMAL
  位于 `canonical_store.rs:132-140`，但 `synchronous` 是 connection-local；
  活体新连接复核为 `journal_mode=wal, synchronous=2(FULL)`。
- **修复：** 先为热路径提供有上限的 `load_items_after(limit)`；连接管理
  沿用仓库既有模式选择复用或小型池，但每个实际读写连接都必须应用完整
  PRAGMA。不要为修一个查询无条件引入新的连接池依赖。
- **验收：** 10k item 会话按页读取且单次结果有界；实际写连接报告
  WAL/NORMAL/busy timeout；并发读写无 `database is locked` 回归。
- **实现证据：** `SessionReadStore` 新增兼容的 bounded page contract
  （`crates/contracts/src/types/session.rs:323-341`），canonical SQLite adapter 用
  `ORDER BY sequence LIMIT` 原生实现（`crates/adapters/sqlite/src/session/canonical_store.rs:490-510`）。
  BUG-008 的同步热路径以 256 条为一页循环并逐页推进 durable watermark
  （`crates/runtime/src/session_service.rs:155-180,221-245`），不再一次物化全部
  后缀。每个 file-backed operation connection 现在都重新应用 busy timeout、
  WAL 和 NORMAL（`crates/adapters/sqlite/src/session/canonical_store.rs:79-85,132-139`）。
  `contracts` 209/209、`runtime` 209/209、`adapters-sqlite` 91/91 通过；10k
  安装态分页、connection-local PRAGMA 与并发锁等待仍待部署验证。

### BUG-031 会话列表 N+1 —— 每行新开一条 SQLite 连接

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/runtime/src/session_service.rs:311-324`
- **修复：** 扩展 store 的批量读取契约，让 session records 与 owner 在
  一次查询/一次连接内返回（JOIN 或批量 IN 均可由 adapter 决定）；Runtime
  层不越过 store abstraction 直接拼 SQLite SQL。
- **实现证据：** `SessionReadStore::list_sessions_with_principal` 保留了非 SQLite
  store 的兼容默认实现（`crates/contracts/src/types/session.rs:346-358`）；
  canonical adapter 用一次 `LEFT JOIN` 和一次连接返回 session+owner
  （`crates/adapters/sqlite/src/session/canonical_store.rs:526-547`）。Runtime 列表
  路径只消费该抽象，不包含 SQLite SQL
  （`crates/runtime/src/session_service.rs:376-393`）。`contracts` 209/209、
  `runtime` 209/209、`adapters-sqlite` 91/91 通过；安装态会话列表延迟与连接数
  仍待部署验证。

### BUG-032 max_iterations=0 语义冲突 —— 定时炸弹

- **状态：** `Confirmed` + `Product decision`
- **位置：**
  - `crates/cognit/src/harness/factory.rs:233-240`：`0` 解释为"未设置
    → 默认 50"（237 行 `if config.max_iterations == 0`）；
  - `crates/cognit/src/harness/linear/mod.rs:411-414`：`0` 解释为
    "无限"（414 行 `== 0 ||` 放行）。
- **证据：** 当前用户配置值为 `max_iterations=0`，最终行为取决于装配路径
  走哪一侧。factory 侧把 0 变成默认值，若哪天直连 linear 侧就变成
  无限循环。
- **待裁决：** 统一语义，但本文不替产品决定 `0` 是“默认”还是“无限”。
  若需要同时表达两者，应使用显式 enum/Option，而不是让同一个整数哨兵
  承担两种语义。当前用户配置 `~/.config/aletheon/config.toml:7` 确为 0，
  变更前必须给出迁移行为，不能静默改变有效预算。
- **验收：** 两侧单测断言同一输入产生同一迭代上限解释。

### BUG-033 原描述：事件流首 delta 被 live-overlay 守卫丢弃（已排除）

- **状态：** `Excluded`
- **代码事实：** `crates/interact/src/tui/reducer.rs:194-215` 的 guard 仅在
  durable assistant 已经存在且 sequence 不早于 live overlay 时移除
  ephemeral 副本；代码没有“overlay 未就绪”分支。
- **处理：** 从修复批次移除。若再次观察到缺首字，需保存 live delta、
  protocol cursor、durable item sequence 和 rendered frame 后重新立项。

### BUG-034 reducer 终态空 item / 重连丢 Streaming 项

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/interact/src/tui/reducer.rs:742-750`
  （`Completed` 带 `item: None` 留下卡死的 Streaming 条目）、
  `crates/interact/src/tui/reducer.rs:139-149,762-773`（重连清除所有
  `status=Streaming` 项，不区分 durable/ephemeral）
- **修复：** `Completed + item=None` 是协议违约：显示 typed 错误并触发
  一次受控 snapshot reload，不能伪造 Completed 内容。重连只删除明确以
  `live:`/`local:` 标识的 ephemeral 项；durable Streaming 项由 snapshot
  权威重建。
- **实现证据：** 单事件与分页路径都检测空 Completed payload，记录可见协议
  错误并触发权威 snapshot reload，而不合成 completed item
  （`crates/interact/src/tui/reducer.rs:121-129,757-829`）；重连清理现在只匹配
  `live:`/`local:` ID，保留 durable Streaming 项直到 snapshot 对齐
  （`crates/interact/src/tui/reducer.rs:832-840`）。`interact` lib suite 183/183
  通过；确定性违约 fixture 仍需测试源码授权。

### BUG-035 CJK 提交固定等待至少 100ms —— 总延迟待量化

- **状态：** `Needs evidence`
- **位置：** CJK 输入固定延迟在
  `crates/interact/src/tui/app/key_handler.rs:821-836`，到期检查在
  `crates/interact/src/tui/app/lifecycle.rs:250-261`；poll cadence 为
  50/200ms（`lifecycle.rs:266-273`）。
- **证据边界：** 代码至少引入 100ms 延迟，实际还受下次循环和同步持久化
  影响；原“100-350ms 实测”没有保存逐键时间线。
- **下一步：** 在 fcitx5 下记录 Enter、pending 到期、submit 开始和首个
  receipt 的单调时间；先分离 IME 安全延迟、event-loop poll 和 BUG-036
  fsync，再决定是否能缩短或只对仍在 composition 的输入延期。

### BUG-036 每次提交在 UI 线程同步 fsync 输入态

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/interact/src/tui/input.rs:205-210`
  （`fs::write + sync_all` 阻塞事件循环）
- **修复：** 改异步后台写（tokio::fs + 防抖），UI 线程零磁盘 IO。
- **实现证据：** UI 只序列化并替换最新 pending snapshot；单个后台任务使用
  `tokio::fs` 写临时文件、`sync_all` 后原子 rename，退出时排空
  （`crates/interact/src/tui/model.rs:140-142,489-495`、
  `crates/interact/src/tui/app/lifecycle.rs:398-427`、
  `crates/interact/src/tui/input.rs:171-246`）。持久化与敏感信息回归通过。

### BUG-037 首帧前 await 会话初始化 —— daemon 慢则黑屏

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/interact/src/tui/app/lifecycle.rs:204-217`
- **修复：** 先渲染框架（loading 态），init 完成后填充。
- **实现证据：** 首个 daemon RPC 前绘制 presentation-only loading frame，Session
  identity 仍保持 unset；初始化成功后才标记 connected 并进入原循环
  （`crates/interact/src/tui/app/lifecycle.rs:199-239`）。

### BUG-038 重连内联 3×100ms 冻结 UI

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/interact/src/tui/app/lifecycle.rs:499-505`
- **修复：** 重连移入后台任务，UI 显示重连状态。
- **实现证据：** 与 BUG-018 共用 controller-owned background reconnect；TUI 仅在
  JoinHandle 已完成时 await 结果（`crates/interact/src/tui/controller.rs:49-104`）。

### BUG-039 pending_events 无界 —— 客户端无背压

- **状态：** `Implemented (uncommitted; installed-runtime pending)`
- **位置：** `crates/gateway/src/client/mod.rs:240,332-334`
  （`VecDeque` 无上限）
- **修复：** 优先让请求响应与事件消费并行，避免等待 response 时把所有
  Event 堆进本地队列；若仍需缓冲，必须有界。`TurnSettled`、approval 和
  session lifecycle 不得丢弃；当前 Event schema 没有“心跳类事件”，不能
  用不存在的类别作为淘汰策略。仅对语义允许合并的 Progress 做显式聚合，
  并记录丢弃/合并计数。
- **实现证据：** interleaved queue 总上限 256，并为 settlement/approval/session
  lifecycle 预留 64 个槽；通用 `client_event` Progress 不做不安全的 last-value
  合并。压力溢出返回 typed `EventBufferOverflow` 并要求重连；关键事件在满队列
  优先淘汰 Progress，累计 drop count
  （`crates/gateway/src/client/mod.rs:26-28,350-390`、
  `crates/gateway/src/protocol/mod.rs:594`）。容量测试证明 terminal 保留；gateway
  client-feature suite 33/33 通过。

### BUG-040 原描述：流式期间滚动锚定尾部、无法回看（已排除）

- **状态：** `Excluded`
- **代码事实：** `crates/interact/src/tui/app/key_handler.rs:904-935` 已维护
  `conversation_scroll`，`crates/interact/src/tui/task_console.rs:250-254`
  从 tail offset 减去该值，live delta 路径也未重置它。
- **处理：** 从修复批次移除；只有带 frame 和按键序列的复现证明滚动仍
  被重置时才重新开放。

### BUG-041 大型会话完整快照超过客户端 1MB 帧限制

- **状态：** `Fixed (uncommitted, installed-runtime verified 2026-08-28)`
- **原始证据：** 153-item、约 500KB 持久化内容的真实会话恢复时，Gateway
  在同一 JSON frame 中同时返回完整 `SessionReadSnapshot.items` 和重复的
  event page，客户端报 `wire frame exceeds the configured limit`。
- **修复：** TUI 请求分页投影；服务端返回 item-free baseline 加有序 event
  page。事件页同时限制 256 条和 512KiB，客户端按 authoritative cursor
  增量合并，不再每页清空历史或下载完整 items。
- **实现证据：** baseline contract
  `crates/contracts/src/protocol/client.rs:1098-1108`；分页查询选择
  `crates/gateway/src/protocol/mod.rs:354-363`；服务端响应拆分
  `crates/aletheon/src/daemon/handler/typed_gateway.rs:820-875`；字节预算
  `crates/runtime/src/session_service.rs:23-24,392-425`；TUI baseline/page 合并
  `crates/interact/src/tui/reducer.rs:291-324`、
  `crates/interact/src/tui/response.rs:542-623`。
- **安装态验证：** `/usr/bin/aletheon` 通过官方用户 socket 成功恢复此前失败
  的同一个 153-item 会话；frame 稳定且无 frame/schema 错误。20 次鼠标上滚
  724ms 内稳定，反向滚动 735ms 内恢复初始尾部。部署后真实 LLM 请求首次
  因 90 秒执行期限取消，随后 180 秒重试成功（17.87 秒、无 provider retry）；
  该首次 deadline 必须保留为独立运行时证据，不计作分页通过依据。

---

## 7. P4 — 数据卫生与证据治理（低优先级）

| ID | 状态 | 问题 | 下一步 |
|----|------|------|--------|
| HYG-001 | `Product decision` | `extension-events.db` 822MB、`events.db` 330MB、`protocol-events-v1.db` 176MB 尚无统一保留契约 | 逐库标明恢复权威、审计要求和可重建性后再定 TTL/容量；禁止统一盲删 |
| HYG-002 | `Confirmed` + `Product decision` | `care_modulation_traces` 2744 行无 prune，写入路径见 `crates/dasein/src/core/mod.rs:431-469` | 明确保留窗口与聚合需求，再实现 prune |
| HYG-003 | `Needs evidence` | 多个 SQLite store 仍为 rollback journal 或每连接 FULL synchronous；已确认 `self_field.db`/`conscious_workspace.db` 为 `delete`，`sessions-v1.db` 新连接为 `wal/full` | 盘点各库读写频率与连接生命周期，只优化热库；不要把 WAL 当作所有库的通用答案 |
| HYG-004 | `Confirmed` | 原审计没有为 journal/DB/时延样本保存不可变原始证据，已出现 130682/100682ms 冲突 | 后续每批在 `docs/testing/` 或批准的证据目录保存命令、时间窗、host、commit、输出摘要和原始 artifact locator |

旧版表中的 `/copy`、末尾反斜杠和 submit 错误恢复项已分别并入
BUG-027b、BUG-027d、BUG-002，不再重复立项；本节 HYG-003/004 是本轮
重新校准后的新编号含义。

---

## 8. 修复批次与依赖关系

```text
Gate 0  冻结只读基线 + 裁决 Product decision
   |
   v
Batch 1 OPS-001：已部署 f4b22ec8 dirty candidate；后续批次仍需重新部署
   |
   +--> Batch 2 P0：BUG-001/002/004（BUG-003 等待产品裁决）
   |
   +--> Batch 3 P0.5：BUG-005 防再增长 -> 安装态验证 -> 批准后清库
   |                  BUG-007/008/021 作为同一性能批次
   |
   +--> Batch 4 P1：BUG-009/010/011；先补 BUG-012 取证
   |
   +--> Batch 5 P2：BUG-013/015/017/017b；BUG-014/016 先基准后决策
   |
   +--> Batch 6 P2.5：BUG-018/019/022/024/025/026/027a-d/034/036/037/038
   |                   BUG-020/023/035 仅在复测确认后加入
   |
   `--> Batch 7 P3/P4：BUG-029/030/031/039 + 已裁决的数据保留项
                       BUG-028 先做因果取证；BUG-032 先裁决 0 语义

Excluded：BUG-033、BUG-040
```

**关键依赖说明：**

1. **先冻结当前只读基线，再做 OPS-001**：部署会改变运行事实；部署前
   保存 DB 大小/行数、服务 PID/NRestarts、安装 SHA 和代表性真实请求。
2. **BUG-005 先阻止再增长，再做 destructive cleanup**：本机仍有约
   713GiB 空闲，不需要冒险做紧急清理。先部署 prune 修复并验证，再停写入
   方、备份、清理和 VACUUM；任何 DB 删除都需另行批准。
3. **BUG-009 与 BUG-011/014 联动**：错误透传修好后，记忆提案失败率
   数据才可信；并发分池影响提案排队时长。
4. **BUG-023 先部署再评估**：`c9f30522` 可能已覆盖部分问题，避免重复改。
5. **BUG-008/007/021 是同一性能故事的三面**：服务端增量同步 + WAL +
   客户端停止全量 reload，三者齐做才能根治"越用越卡"。
6. **BUG-017b/018/019/020/034 共享终态与重连语义**：先定义 authoritative
   cursor/terminal，再改 UI；任何 watchdog 都不能伪造成功终态。
7. **BUG-013/014/016 共享 provider 时间预算**：重试、共享 admission、
   request 和 stream idle 必须分别计量但服从同一端到端预算。

---

## 9. 验收与防护网（全批次通用）

1. **验证与测试授权分离**：每项必须有修复前失败、修复后通过的确定性
   验证；优先运行既有测试。新增/修改测试源码、fixture、snapshot、test
   helper 或构建 target 需要单独明确授权，不能由本文自动授权。
2. **核心范围门禁**：BUG-009/013/014/015/016 涉及 Agent control、模型
   推理或 provider 调度，BUG-028/029 涉及 terminal fence/运行时控制。
   实现前必须重新列出准确文件与 symbol、说明 feature-local 修复为何不足、
   描述运行风险，并获得对应核心范围的明确批准。普通“开始修 bug”不自动
   授权这些核心文件。
3. **构建纪律**：所有 Rust build/check/test/lint/doc 必须通过
   `bash scripts/cargo-agent.sh <cargo arguments>`，使用能覆盖改动的最窄
   package/target；只有集成验收负责人运行 workspace-wide 检查。格式检查
   使用 `bash scripts/cargo-agent.sh fmt --all -- --check`。
4. **安装态验收**：涉及工具、配置、持久化、IPC、daemon 或客户端行为的
   批次，只有以下条件全部满足才可关闭：
   - 经批准执行 `sudo bash scripts/aletheon.sh deploy` 成功；
   - `target/release/aletheon`、`/usr/bin/aletheon`、运行中的 machine core、
     user daemon 和 memory-agent 的 `/proc/<pid>/exe` SHA-256 完全一致；
   - 部署前后分别记录 system/user unit、MainPID、ActiveState、NRestarts，
     观察窗口内 restart counter 稳定；
   - 使用 `/usr/bin/aletheon` 和 official user socket 完成真实 LLM 请求，
     并核对 rendered frame、session durable evidence 与 daemon logs；
   - 触及 model-controlled routing 或 tool arguments 时，使用相同安装态
     连续完成 3 次真实 TUI 运行；
   - 任一 `provider_unavailable`、`provider_rejected_request` 或 rendered
     inference error 都使该次真实 TUI 验收失败，monitor PASS 不能覆盖它。
   详细关闭记录沿用 `docs/testing/runtime-correctness.md`。
5. **观测基线**（修复前后使用同一命令、host、时间窗；全部只读）：
   ```bash
   # 磁盘
   du -h ~/.local/state/aletheon/*.db | sort -h
   sqlite3 -readonly ~/.local/state/aletheon/self_field.db \
     ".timeout 5000" \
     "SELECT count(*), max(length(event_prefix_json)) FROM self_snapshots;"
   # journal 告警聚类
   journalctl --user -u aletheon --since "-1h" --output=cat | \
     grep -oE '"message":"[^"]+"' | sort | uniq -c | sort -rn | head
   # turn/provider 指标必须按同一条记录关联，不能把分离的 tail 值拼成一轮
   journalctl --user -u aletheon --since "-1h" --output=cat | \
     grep 'Turn latency breakdown'
   # 所有 aletheon 进程内存与运行时间
   pgrep -x aletheon | xargs -r ps -o pid,rss,etime,user,cmd -p
   ```
6. **指标纪律**：model inference rounds、provider attempts/retries、tool
   calls、active context occupancy、cache usage 和累计 token 分开报告；不能
   用 tool 数降低证明 provider 请求减少，也不能用累计 token 推导上下文
   占用。
7. **提交边界**：仅在用户要求提交/发版后创建 commit；届时按批次形成
   可审查阶段，提交信息包含问题/方案/具体变更。本文不自动授权创建 PR、
   push、部署或任何 GitLab/GitHub 写操作。

---

## 10. 与既有台账的关系

- 本文与 `docs/design/roadmap/runtime-correctness-backlog.md` 并行：backlog
  记录跨批次正确性债务，本文记录可执行条目。RC-001 的文字早于当前
  `ProviderBackpressureConfig` 机器进程级治理；实现 BUG-014 前须重新确认
  多进程/provider 边界，不能直接把两者视为同一问题。
- RC-003 是 monitor settle 判定问题；BUG-019/034 是 TUI projection/reducer
  问题。它们共享 terminal evidence，但属于不同实现和验收对象，不合并
  修复，只共享测试证据。
- `docs/design/README.md` 状态表中 Testing 行的 "long-duration and
  concurrent-session coverage: Partial" 即本文 P3 多项的验收短板，
  P3 批次应补上 soak 测试。

---

## 附录 A：取证快照（2026-08-27）

| 指标 | 值 |
|------|-----|
| 状态目录总量 | ~12GB |
| self_field.db | 9.7GB（self_snapshots 280 行 / 最大 59,106,692 bytes；self_events 103,863 行；auto_vacuum=0） |
| extension-events.db | 822MB |
| events.db | 330MB |
| conscious_workspace.db | 328MB（1153 space / 8037 投影行） |
| protocol-events-v1.db | 176MB |
| sessions-v1.db | 43MB / 3838 会话（`sessions.db` 另为 608KB / 2838 行） |
| daemon | PID 2483, RSS 429584KiB, uptime 约 1d6h（22:55 CST） |
| 安装版本 | `2369d752b788740e9ddbe24b38e80f160637966e-dirty-0e734d57984b0917`（仓库 `f4b22ec8`） |
| journal 高频告警 | internal runtime error ×9（记忆提案）；conscious read failed ×10+ |
| turn 时延历史样本 | 784119ms/13r; 194229ms/13r; 121989ms/8r；原审计第三条在正文/附录分别为 130682/100682ms，当前 journal 无法复核，已排除 |
| 启动崩溃 | 2026-08-26 16:42 ENOENT ×2 → restart-loop |

## 附录 B：审计覆盖与方法

- **主会话**：dasein/agora/mnemosyne/agent_control 源码审读 + 活体
  DB/journal/doctor/ps 取证（§0、§1-3、附录 A）。
- **daemon/kernel 历史审计**：14 组候选经本轮主会话重新核对后分为
  Confirmed/Needs evidence/Excluded，不再把候选数称作“已确认 bug 数”。
  其“验证无问题清单”（canonical 建库 WAL、stale socket
  清理、turn 终态栅栏、kernel 进程监督、版本失配 typed error、
  mnemosyne spawn_blocking + 有界 recall cache）未列入问题。
- **TUI 历史审计**：26 组候选已重新分类；BUG-033/040 因与当前代码不符
  标为 Excluded，BUG-027 拆为四个独立条目。无问题清单记录于 §5 末尾。
- 所有完整路径引用基于 `f4b22ec8`；2026-08-27 修订时重新 grep。短路径
  仅用于同一条目内紧邻引用，不作为独立 locator。
