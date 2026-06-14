# OS-Agent: 融合操作系统的自主智能体

> 一个将 AI Agent 深度融入操作系统内核与系统服务的架构设计方案。
> 目标：让 Agent 成为操作系统的"第二大脑"，而不是一个 App。

**目标平台:** Linux (Arch Linux 为主) / Android / 嵌入式开发板
**创建日期:** 2026-06-06
**作者:** aurobear

---

## 目录

- [1. 项目愿景](#1-项目愿景)
- [2. 为什么要做这件事](#2-为什么要做这件事)
- [3. 与现有方案的本质区别](#3-与现有方案的本质区别)
- [4. 系统架构总览](#4-系统架构总览)
- [5. Linux 平台深度设计 (Arch Linux)](#5-linux-平台深度设计-arch-linux)
- [6. Android 平台设计](#6-android-平台设计)
- [7. 嵌入式/开发板设计](#7-嵌入式开发板设计)
- [8. 安全模型](#8-安全模型)
- [9. 认知引擎设计](#9-认知引擎设计)
- [10. 记忆系统](#10-记忆系统)
- [11. 感知层设计](#11-感知层设计)
- [12. 执行层设计](#12-执行层设计)
- [13. 混合推理架构 (本地+云端)](#13-混合推理架构-本地云端)
- [14. 实现路线图](#14-实现路线图)
- [15. 技术选型](#15-技术选型)
- [16. 开放问题](#16-开放问题)

---

## 1. 项目愿景

### 核心理念

```
Agent 不是一个"聊天窗口"，
Agent 应该是操作系统的"第二大脑" —

  有感知、有记忆、有决策、有执行，
  永远在线，和系统共生。
```

### 设计目标

| 目标 | 描述 |
|------|------|
| **系统级存在** | Agent 以 daemon/service 形式常驻，是系统的一部分而非用户主动启动的 App |
| **全栈感知** | 从内核事件到用户行为，Agent 能"看见"系统的一切 |
| **自主决策** | 基于感知和记忆，Agent 能自主规划和执行任务 |
| **安全可控** | 分级权限策略，关键操作可审计可回滚 |
| **离线优先** | 本地推理优先，复杂任务可 Fallback 到云端 |
| **跨平台** | Linux PC / Android / 嵌入式开发板统一架构 |

---

## 2. 为什么要做这件事

### 现状的尴尬

```
当前 "Agent" 的系统集成程度:

OpenAI / Claude API      →  完全在云端，和你的 OS 零关系
GitHub Copilot           →  编辑器插件，不碰系统
Windows Copilot          →  UI 层套壳，不碰内核
macOS Intelligence       →  Siri 换皮，沙箱里
Linux 各种 CLI agent     →  bash 执行器，没有自主意识
Android Assistant        →  云端服务，无法控制系统

没有一家做到: Agent = 操作系统的"第二大脑"
```

### 根本原因

大家把 Agent 当 **App** 做，不是当 **OS 组件** 做。

### 机会

Linux 上，技术条件已经全部具备：
- **eBPF** → 内核级感知
- **systemd** → 生命周期管理
- **FUSE** → 用户态文件系统接口
- **llama.cpp / whisper.cpp** → 本地推理
- **D-Bus** → 进程间通信
- **cgroups/namespaces** → 安全沙箱

**缺的只是一层把这些粘合在一起的 Agent Runtime。**

---

## 3. 与现有方案的本质区别

```
┌──────────────┬──────────────────┬──────────────────────┐
│              │  现有 Agent       │  OS-Agent            │
│              │  (Claude/GPT等)   │  (本项目)             │
├──────────────┼──────────────────┼──────────────────────┤
│ 运行位置      │  云端             │  本地系统服务         │
│ 系统感知      │  无 / 需要工具    │  eBPF + /proc        │
│ 执行能力      │  API 调用         │  直接系统调用         │
│ 持久性        │  会话级           │  永驻 (systemd)      │
│ 记忆          │  上下文窗口       │  持久化存储           │
│ 自主性        │  需要人触发       │  事件驱动自主行动     │
│ 安全          │  平台托管         │  本地策略引擎         │
│ 延迟          │  100ms+ 网络     │  <1ms 本地            │
│ 隐私          │  数据上云         │  数据不出本机         │
│ 依赖          │  必须联网         │  离线可用             │
│ 角色          │  "工具"           │  "系统的一部分"       │
│ 平台          │  跨平台但浅       │  深度融入每个 OS      │
└──────────────┴──────────────────┴──────────────────────┘
```

---

## 4. 系统架构总览

### 跨平台统一架构

```
                    ┌─────────────────────────────────┐
                    │      OS-Agent Core Runtime       │
                    │                                 │
                    │  ┌───────────┐  ┌────────────┐  │
                    │  │ 认知引擎  │  │ 记忆系统    │  │
                    │  │ Planner   │  │ Memory     │  │
                    │  │ Reasoner  │  │ Vector+SQL │  │
                    │  └───────────┘  └────────────┘  │
                    │  ┌───────────┐  ┌────────────┐  │
                    │  │ 安全引擎  │  │ 交互层     │  │
                    │  │ Policy    │  │ CLI/TUI/UI │  │
                    │  │ Sandbox   │  │ API/D-Bus  │  │
                    │  └───────────┘  └────────────┘  │
                    └────────────┬────────────────────┘
                                 │
                    ┌────────────┼────────────────────┐
                    │            │                     │
            ┌───────┴──────┐ ┌──┴──────────┐ ┌───────┴──────┐
            │   Linux      │ │  Android    │ │  嵌入式      │
            │   Adapter    │ │  Adapter    │ │  Adapter     │
            ├──────────────┤ ├─────────────┤ ├──────────────┤
            │ eBPF         │ │ Binder      │ │ GPIO         │
            │ systemd      │ │ AOSP APIs   │ │ I2C/SPI      │
            │ D-Bus        │ │ Accessibility│ │ UART         │
            │ /proc /sys   │ │ Root/ADB    │ │ RTOS hooks   │
            │ FUSE         │ │ NDK         │ │ NPU          │
            │ iptables     │ │ Intent      │ │ 传感器       │
            └──────────────┘ └─────────────┘ └──────────────┘
```

### 核心设计原则

1. **模块化** — 每个能力是一个 plugin，可独立加载/卸载
2. **安全第一** — 默认最小权限，显式授权升级
3. **可观测** — 所有决策可审计可回溯
4. **离线优先** — 本地能做的不依赖云端
5. **渐进式** — 从简单到复杂，每个阶段都有价值
6. **平台抽象** — 核心逻辑与平台无关，通过 Adapter 对接不同 OS

---

## 5. Linux 平台深度设计 (Arch Linux)

### 5.1 系统集成层次

```
┌─────────────────────────────────────────────────────────────┐
│                    用户态 (User Space)                        │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐    │
│  │              Agent Userland Service                  │    │
│  │                                                     │    │
│  │  ┌───────────┐ ┌────────────┐ ┌──────────────────┐ │    │
│  │  │ 推理引擎   │ │ 记忆系统    │ │ 规划/决策引擎    │ │    │
│  │  │ llama.cpp  │ │ SQLite+    │ │ ReAct/Plan&Exec │ │    │
│  │  │ whisper.cpp│ │ ChromaDB   │ │ 任务队列        │ │    │
│  │  └───────────┘ └────────────┘ └──────────────────┘ │    │
│  │  ┌───────────┐ ┌────────────┐ ┌──────────────────┐ │    │
│  │  │ 执行引擎   │ │ 感知引擎    │ │ 对话/交互层      │ │    │
│  │  │ bash exec │ │ inotify    │ │ TUI/WebUI/IPC   │ │    │
│  │  │ D-Bus调用 │ │ netlink    │ │ socket server   │ │    │
│  │  │ systemd   │ │ udev       │ │                  │ │    │
│  │  └───────────┘ └────────────┘ └──────────────────┘ │    │
│  └──────────────────────┬──────────────────────────────┘    │
│                         │ eBPF / ptrace / netlink            │
│  ┌──────────────────────┼──────────────────────────────┐    │
│  │              内核态 (Kernel Space)                    │    │
│  │                      │                               │    │
│  │  ┌──────────┐ ┌─────┴────┐ ┌─────────┐ ┌────────┐  │    │
│  │  │ 文件系统  │ │ 进程调度  │ │ 网络栈   │ │ 设备驱动│  │    │
│  │  │ ext4/btrfs│ │ CFS/BPF  │ │ TCP/IP  │ │ GPIO   │  │    │
│  │  └──────────┘ └──────────┘ └─────────┘ └────────┘  │    │
│  └─────────────────────────────────────────────────────┘    │
│                         │                                    │
│              ┌──────────┴──────────┐                        │
│              │    硬件 (CPU/GPU/RAM) │                        │
│              └─────────────────────┘                        │
└─────────────────────────────────────────────────────────────┘
```

### 5.2 eBPF 感知层

eBPF 是 Linux 的杀手级特性，也是 Agent 融入 OS 的关键通道。
Agent 通过 eBPF 可以安全地在内核里运行沙箱化程序，实现零开销的系统感知。

**监控能力:**

| eBPF Hook Point | Agent 能感知什么 | 用途 |
|---|---|---|
| `sys_enter_openat` | 每个文件的打开操作 | 文件访问模式分析 |
| `sched_process_exec` | 每个进程的创建 | 异常进程检测 |
| `vfs_read/vfs_write` | 文件读写 | 数据流追踪 |
| `tcp_connect/tcp_send` | 网络连接 | 流量分析/安全防护 |
| `tracepoint/*` | 任意内核事件 | 全面系统感知 |

**概念性 eBPF 程序示例:**

```c
// Agent 的"内核感知器" — 监控所有文件打开操作
SEC("tracepoint/syscalls/sys_enter_openat")
int trace_open(struct trace_event_raw_sys_enter *ctx) {
    struct event_t evt = {};
    evt.pid = bpf_get_current_pid_tgid() >> 32;
    evt.ts  = bpf_ktime_get_ns();
    bpf_get_current_comm(&evt.comm, sizeof(evt.comm));

    const char *filename = (const char *)ctx->args[1];
    bpf_probe_read_user_str(&evt.filename, sizeof(evt.filename), filename);

    // 通过 ring buffer 实时推送给 Agent daemon
    bpf_ringbuf_submit(&evt, sizeof(evt));
    return 0;
}
```

**优势:**
- 零拷贝: 数据直接从内核流到 Agent
- 安全: 沙箱化，验证器保证不崩溃内核
- 动态: 运行时加载/卸载，不需要重启

### 5.3 FUSE 虚拟文件系统

Agent 创建一个 FUSE 挂载点，让系统和用户通过文件接口与 Agent 交互：

```
/mnt/agent/                    # Agent 的 FUSE 挂载点
├── context/                   # Agent 的当前上下文
│   ├── focus                 # cat focus → 当前关注什么
│   ├── tasks                 # cat tasks → 任务队列
│   └── memory/               # 记忆目录
│       ├── recent            # 最近的记忆
│       └── long_term         # 长期记忆
├── controls/                  # Agent 的控制接口
│   ├── schedule              # echo "明天9点开会" > schedule
│   ├── notify                # echo "提醒我" > notify
│   └── execute               # echo "编译项目" > execute
├── sensors/                   # Agent 的感知数据
│   ├── screen                # cat screen → 当前屏幕内容
│   ├── network               # cat network → 网络状态
│   └── system                # cat system → 系统状态
└── logs/                      # Agent 的决策日志
    ├── decisions             # Agent 做了什么决定
    └── reasoning             # Agent 为什么这样做
```

**交互方式:**
```bash
# 任何程序都能通过标准文件接口和 Agent 交互
cat /mnt/agent/tasks                          # 读取 Agent 状态
echo "明天9点开会" > /mnt/agent/controls/schedule  # 给 Agent 下达指令
inotifywait /mnt/agent/sensors/               # 监听 Agent 感知
```

### 5.4 systemd 集成

Agent 作为 systemd 服务运行，成为系统公民：

```ini
# /etc/systemd/system/agentd.service
[Unit]
Description=Agent System Service
After=network.target
Wants=llama-server.service
Before=shutdown.target

[Service]
Type=notify
ExecStart=/usr/bin/agentd --config /etc/agent/agent.conf

# 安全沙箱
ProtectSystem=strict
ReadWritePaths=/home /tmp /var/lib/agent
ProtectHome=false
PrivateTmp=true

# 资源限制
CPUQuota=80%
MemoryMax=8G
IOWeight=50

# 能力控制（不是 root 全部权限）
CapabilityBoundingSet=CAP_NET_ADMIN CAP_DAC_OVERRIDE CAP_SYS_PTRACE
AmbientCapabilities=CAP_NET_ADMIN CAP_DAC_OVERRIDE CAP_SYS_PTRACE

# 看门狗 — Agent 死了自动重启
WatchdogSec=30s
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

**systemd 生态 Agent 能接入的子系统:**

| 子系统 | Agent 能做什么 |
|--------|---------------|
| journald | 系统日志实时流 |
| logind | 用户登录/会话管理 |
| machined | 容器/虚拟机管理 |
| networkd | 网络配置和状态 |
| resolved | DNS 解析 |
| timesyncd | 时间同步 |
| tmpfiles | 临时文件清理 |
| timers | 定时任务 |
| D-Bus | 进程间通信总线 |
| polkit | 权限策略 |

### 5.5 用户态系统控制能力

Agent 可以通过标准 Linux 接口控制系统：

```
文件系统控制:
├── inotify — 实时监控任意目录的增删改
├── fswatch/watchdog — 跨平台文件监控
├── find/grep/rg — 文件搜索
├── btrfs snapshot — 文件系统快照
└── Agent 可以: 发现新文件→自动分类, 敏感文件变动→告警

进程控制:
├── systemd — 服务启停、依赖管理
├── cgroups — 资源限制 (CPU/内存/IO)
├── nice/renice — 调度优先级
└── Agent 可以: 自动优化资源分配, 智能调度任务

网络控制:
├── iptables/nftables — 防火墙规则
├── ss/netstat — 连接监控
├── NetworkManager D-Bus — 网络切换
└── Agent 可以: 自动切换网络, 流量分析, 安全防护

硬件控制:
├── /sys/class — CPU频率, 风扇, 背光, LED
├── /dev/input — 键盘鼠标事件
├── udev rules — 设备热插拔响应
└── Agent 可以: 自动调节能耗, 设备即插即管理
```

### 5.6 Arch Linux 的独特优势

```
为什么 Arch 特别适合做 OS-Agent:

1. 滚动更新 — 内核和工具链永远最新，eBPF 支持最好
2. AUR — llama.cpp, whisper.cpp, chromadb 等都有现成包
3. KISS 哲学 — 系统简洁，Agent 接入的干扰最少
4. pacman — 优秀的包管理，Agent 可以直接集成
5. systemd — 全功能 systemd，Agent 可以深度集成
6. ArchWiki — 无与伦比的文档，开发参考极方便
7. 用户群体 — Arch 用户天然愿意折腾和定制
```

---

## 6. Android 平台设计

### 6.1 Android 的特殊性

```
Android vs Linux 的关键差异:

┌───────────────┬──────────────────┬──────────────────┐
│               │ Linux (Arch)     │ Android          │
├───────────────┼──────────────────┼──────────────────┤
│ 内核           │ 主线 Linux       │ 修改版 Linux      │
│ IPC           │ D-Bus/Socket     │ Binder           │
│ 进程模型      │ 自由             │ 沙箱化            │
│ 权限模型      │ root/uid         │ 权限声明+运行时   │
│ 系统服务      │ systemd          │ System Server    │
│ 包管理        │ pacman/apt       │ PackageManager   │
│ 无 root 访问  │ 大部分可以       │ 严格限制          │
│ 用户交互      │ Terminal/桌面    │ 触屏/通知         │
└───────────────┴──────────────────┴──────────────────┘
```

### 6.2 Android Agent 架构

```
┌─────────────────────────────────────────────────────────┐
│                    Android 系统                          │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │                Agent App Layer                     │  │
│  │                                                   │  │
│  │  ┌─────────────┐  ┌──────────────┐               │  │
│  │  │ Agent       │  │ Notification │               │  │
│  │  │ Foreground  │  │ Listener     │               │  │
│  │  │ Service     │  │ Service      │               │  │
│  │  └──────┬──────┘  └──────┬───────┘               │  │
│  │         │                │                        │  │
│  │  ┌──────┴────────────────┴──────────────────────┐ │  │
│  │  │           Agent Core Engine                   │ │  │
│  │  │  ┌──────────┐ ┌───────────┐ ┌─────────────┐ │ │  │
│  │  │  │ 认知引擎  │ │ 记忆系统   │ │ 任务规划器  │ │ │  │
│  │  │  │ LLM/云端  │ │ SQLite    │ │ ReAct      │ │ │  │
│  │  │  └──────────┘ └───────────┘ └─────────────┘ │ │  │
│  │  └─────────────────────────────────────────────┘ │  │
│  └───────────────────────────┬───────────────────────┘  │
│                              │                           │
│  ┌───────────────────────────┴───────────────────────┐  │
│  │              Android 系统接口                       │  │
│  │                                                   │  │
│  │  ┌────────────┐ ┌───────────┐ ┌────────────────┐ │  │
│  │  │Accessibility│ │ Intent    │ │ ContentResolver│ │  │
│  │  │Service     │ │ System    │ │ (文件/联系人等) │ │  │
│  │  │(屏幕感知)  │ │ (应用通信) │ │                │ │  │
│  │  └────────────┘ └───────────┘ └────────────────┘ │  │
│  │  ┌────────────┐ ┌───────────┐ ┌────────────────┐ │  │
│  │  │Notification│ │ Device    │ │ Storage        │ │  │
│  │  │Listener    │ │ Admin     │ │ Access         │ │  │
│  │  │(通知监听)  │ │ (设备管理) │ │ Framework      │ │  │
│  │  └────────────┘ └───────────┘ └────────────────┘ │  │
│  └───────────────────────────────────────────────────┘  │
│                              │                           │
│  ┌───────────────────────────┴───────────────────────┐  │
│  │           Root/ADB 扩展能力 (可选)                  │  │
│  │                                                   │  │
│  │  ┌────────────┐ ┌───────────┐ ┌────────────────┐ │  │
│  │  │ Root Shell │ │ System    │ │ Kernel Module  │ │  │
│  │  │ (su)       │ │ Properties│ │ (eBPF 等)     │ │  │
│  │  └────────────┘ └───────────┘ └────────────────┘ │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### 6.3 Android 无 Root 感知能力

即使不 Root，Android Agent 仍有丰富的感知和控制能力：

```kotlin
// 感知层 — 无需 Root

// 1. 屏幕内容感知 (AccessibilityService)
class AgentAccessibilityService : AccessibilityService() {
    override fun onAccessibilityEvent(event: AccessibilityEvent) {
        // 获取当前屏幕上的所有节点
        val rootNode = rootInActiveWindow
        val screenContent = extractText(rootNode)
        // 发送给 Agent 认知引擎
        AgentEngine.onScreenUpdate(screenContent, event)
    }
}

// 2. 通知感知 (NotificationListenerService)
class AgentNotificationListener : NotificationListenerService() {
    override fun onNotificationPosted(sbn: StatusBarNotification) {
        // 捕获所有通知
        val title = sbn.notification.extras.getString(Notification.EXTRA_TITLE)
        val text = sbn.notification.extras.getString(Notification.EXTRA_TEXT)
        AgentEngine.onNotification(title, text, sbn.packageName)
    }
}

// 3. 应用使用感知 (UsageStatsManager)
val usageStatsManager = getSystemService(USAGE_STATS_SERVICE) as UsageStatsManager
val stats = usageStatsManager.queryUsageStats(...)
// Agent 知道用户在用什么 App，用了多久

// 4. 位置感知
// 5. 传感器感知 (加速度计/陀螺仪/气压计等)
// 6. 剪贴板感知
// 7. 媒体播放状态感知
```

### 6.4 Android 无 Root 执行能力

```kotlin
// 执行层 — 无需 Root

// 1. 发送通知
AgentNotifier.send(context, "Agent 提醒", "你的快递到了")

// 2. 启动应用
val intent = packageManager.getLaunchIntentForPackage("com.example.app")
startActivity(intent)

// 3. 模拟点击 (通过 AccessibilityService)
fun performClick(x: Int, y: Int) {
    val path = Path().apply { moveTo(x.toFloat(), y.toFloat()) }
    val gesture = GestureDescription.Builder()
        .addStroke(GestureDescription.StrokeDescription(path, 0, 100))
        .build()
    dispatchGesture(gesture, null, null)
}

// 4. 自动化操作 (Tasker 风格)
// - 定时任务 (AlarmManager/WorkManager)
// - 条件触发 (地理围栏/充电状态/WiFi连接)
// - 系统设置修改 (Settings.System, 需要 WRITE_SETTINGS 权限)

// 5. 语音交互
// - TTS 语音播报
// - STT 语音输入 (whisper.cpp 或系统 STT)

// 6. Intent 发送
// - 打电话、发短信、打开网页、分享内容
// - 系统设置页面跳转
```

### 6.5 Android Root 扩展能力

```kotlin
// Root 后的额外能力

// 1. 直接 Shell 执行
Runtime.getRuntime().exec("su -c 'iptables -A INPUT -s 1.2.3.4 -j DROP'")

// 2. 系统属性读写
// getprop / setprop

// 3. 应用强制停止/清除数据
// am force-stop / pm clear

// 4. 系统级文件访问
// /data/data/*, /system/* 等

// 5. 内核模块加载 (如果有内核源码)
// insmod / rmmod

// 6. 网络层控制
// iptables / ip rule / tc (流量控制)

// 7. 进程优先级调整
// renice / ionice / cgroups
```

### 6.6 Android 特有的 Agent 场景

```
Android Agent 的独特价值:

智能通知管理:
├── 自动分类通知 (重要/可以等/垃圾)
├── 智能摘要长通知
├── 跨 App 信息关联
└── "你说过要关注快递，这是快递通知"

屏幕内容理解:
├── 看到你在搜索什么，主动提供帮助
├── 看到错误信息，自动诊断
├── 跨 App 数据搬运 (从这个 App 复制到那个)
└── "我看到你在看机票，3天前这个航班更便宜"

自动化工作流:
├── 到公司自动连 WiFi、开勿扰
├── 充电时自动备份照片
├── 收到特定消息自动回复
└── "你每天 9 点都会做这件事，我帮你自动化了"

省电优化:
├── 学习使用模式，智能休眠后台 App
├── 预测下一使用的 App，提前预加载
└── "根据你的使用习惯，我调整了电池策略"
```

---

## 7. 嵌入式/开发板设计

### 7.1 平台选型

| 开发板 | NPU 算力 | 适合场景 | 成本 | 推荐度 |
|--------|---------|----------|------|--------|
| RK3588 (Rock5) | 6 TOPS | 本地 7B 量化模型 | ~¥500 | ⭐⭐⭐⭐⭐ |
| Jetson Orin Nano | 40 TOPS | 视觉+语言多模态 | ~¥2500 | ⭐⭐⭐⭐ |
| Khadas Mind | NPU | 轻量 Agent | ~¥800 | ⭐⭐⭐ |
| ESP32 + 云端 | 无 | 感知+执行, 云端思考 | ~¥30 | ⭐⭐⭐ |
| RISC-V (LicheeRV) | 可选 | 开源硬件 + AI 扩展 | ~¥200 | ⭐⭐⭐⭐ |

### 7.2 嵌入式 Agent 架构

```
┌──────────────────────────────────────────────────────┐
│                嵌入式 Agent 系统                       │
│                                                      │
│  ┌────────────────────────────────────────────────┐  │
│  │              Agent Runtime (Rust/C)             │  │
│  │                                                │  │
│  │  ┌──────────┐ ┌──────────┐ ┌───────────────┐  │  │
│  │  │ 本地推理  │ │ 感知融合  │ │ 执行控制器    │  │  │
│  │  │ llama.cpp │ │ 传感器   │ │ GPIO/电机     │  │  │
│  │  │ Qwen3-3B │ │ 摄像头   │ │ 继电器/LED    │  │  │
│  │  │ GGUF量化 │ │ 麦克风   │ │ UART/I2C     │  │  │
│  │  └──────────┘ └──────────┘ └───────────────┘  │  │
│  │  ┌──────────┐ ┌──────────┐ ┌───────────────┐  │  │
│  │  │ 通信层    │ │ 安全层    │ │ 存储层        │  │  │
│  │  │ MQTT     │ │ 看门狗   │ │ Flash/SD卡    │  │  │
│  │  │ HTTP     │ │ 权限分级 │ │ 记忆持久化    │  │  │
│  │  │ BLE/WiFi │ │ 安全启动 │ │ 日志系统      │  │  │
│  │  └──────────┘ └──────────┘ └───────────────┘  │  │
│  └────────────────────────────────────────────────┘  │
│                        │                              │
│  ┌─────────────────────┴──────────────────────────┐  │
│  │        Linux (Buildroot/Yocto) 或 RTOS         │  │
│  └────────────────────────────────────────────────┘  │
│                        │                              │
│  ┌─────────────────────┴──────────────────────────┐  │
│  │              硬件层 (SoC + 外设)                 │  │
│  │                                                │  │
│  │  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌─────┐ │  │
│  │  │ NPU  │ │ GPU  │ │ GPIO │ │ SPI  │ │ ADC │ │  │
│  │  │ 6TOPS│ │ Mali │ │ 40pin│ │ I2C  │ │ PWM │ │  │
│  │  └──────┘ └──────┘ └──────┘ └──────┘ └─────┘ │  │
│  └────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────┘
```

### 7.3 嵌入式 SDK 设计

```
agent-sdk/
├── libagent.a               # 静态库 (嵌入式) / .so (Linux)
├── include/
│   ├── agent.h              # C API
│   ├── agent_perception.h   # 感知接口
│   ├── agent_execution.h    # 执行接口
│   └── agent_memory.h       # 记忆接口
├── bindings/
│   ├── python/              # Python bindings
│   ├── rust/                # Rust bindings
│   └── micropython/         # MicroPython (ESP32)
├── plugins/
│   ├── sensor_gateway/      # 传感器接入
│   ├── device_control/      # GPIO/电机/继电器
│   ├── camera_vision/       # 摄像头视觉
│   ├── audio_io/            # 麦克风/扬声器
│   ├── network_comm/        # WiFi/BLE/MQTT
│   └── local_llm/           # 本地推理引擎
├── tools/
│   ├── agent-flash          # 固件烧录工具
│   ├── agent-config         # 配置工具
│   └── agent-monitor        # 远程监控
└── examples/
    ├── smart_home/          # 智能家居示例
    ├── robot_arm/           # 机械臂示例
    └── edge_monitor/        # 边缘监控示例
```

### 7.4 混合推理架构

```
嵌入式设备的算力有限，采用混合推理:

┌─────────────────────────────────────────────────┐
│                 混合推理决策树                     │
│                                                 │
│  输入: 用户请求 / 传感器事件                      │
│      │                                          │
│      ▼                                          │
│  ┌─────────────────┐                            │
│  │ 意图分类         │                            │
│  │ (本地小模型 1B)  │                            │
│  └────────┬────────┘                            │
│           │                                     │
│     ┌─────┴─────┬──────────┬──────────┐         │
│     ▼           ▼          ▼          ▼         │
│  简单指令    中等任务    复杂推理    专业任务      │
│  本地处理    本地7B     云端API    专业服务       │
│  <10ms      100-500ms  1-5s      5-30s         │
│  开灯/关灯  写代码     数学证明    视频生成       │
└─────────────────────────────────────────────────┘
```

---

## 8. 安全模型

### 8.1 核心安全理念

> Agent 能控制系统 = 有物理破坏能力，安全不是可选项，是前提条件。

### 8.2 权限分级策略

```
┌──────────────────────────────────────────────────────┐
│                                                      │
│  L0 - 自动执行 (Agent 自己决定, 无需通知)              │
│  ├── 读取文件/目录                                    │
│  ├── 查看系统状态 (/proc, /sys)                       │
│  ├── 搜索信息 (grep, find, rg)                        │
│  ├── 日程提醒和通知                                    │
│  └── 非敏感的文件操作 (工作目录内)                      │
│                                                      │
│  L1 - 通知后执行 (做了告诉用户)                        │
│  ├── 安装/更新软件包                                   │
│  ├── 修改配置文件                                      │
│  ├── 管理 systemd 服务                                │
│  ├── 网络配置变更                                      │
│  └── 资源消耗较大的操作                                │
│                                                      │
│  L2 - 需要确认 (做之前先问)                            │
│  ├── 删除文件 (非临时文件)                             │
│  ├── 修改系统关键配置                                  │
│  ├── 执行需要 sudo 的命令                              │
│  ├── 修改防火墙规则                                    │
│  └── 访问密码/密钥                                     │
│                                                      │
│  L3 - 禁止 (无论什么情况都不做)                        │
│  ├── rm -rf /                                        │
│  ├── 修改内核模块                                      │
│  ├── 关闭安全服务                                      │
│  └── 未经验证的远程代码执行                             │
│                                                      │
└──────────────────────────────────────────────────────┘
```

### 8.3 策略配置

```yaml
# /etc/agent/policy.yaml
security:
  default_level: L1
  auto_approve_timeout: 0  # 永不自动批准 L2+
  audit_log: /var/log/agent/audit.jsonl
  rollback_enabled: true   # 关键操作前自动快照

  rules:
    - pattern: "package install"
      level: L1
      allowed_aur: false  # AUR 包需要确认
    - pattern: "rm *"
      level: L2
    - pattern: "systemctl enable *"
      level: L1
    - pattern: "iptables *"
      level: L2

  sandbox:
    enabled: true
    method: bubblewrap       # 轻量级沙箱
    network_default: false   # 沙箱内默认无网络
    home_readonly: true      # 沙箱内 home 只读
```

### 8.4 安全架构

```
┌─────────────────────────────────────────────────────────┐
│                    安全层架构                             │
│                                                         │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐ │
│  │ 策略引擎     │  │ 沙箱执行器    │  │ 审计日志       │ │
│  │ (OPA/Casbin)│  │ (bubblewrap) │  │ (JSONL)       │ │
│  │             │  │              │  │               │ │
│  │ 规则匹配    │  │ namespace    │  │ 所有决策记录   │ │
│  │ 权限判定    │  │ seccomp      │  │ 可回溯分析     │ │
│  │ 动态更新    │  │ cgroups      │  │ 异常检测       │ │
│  └─────────────┘  └──────────────┘  └────────────────┘ │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐ │
│  │ 回滚引擎     │  │ 看门狗       │  │ 加密存储       │ │
│  │             │  │              │  │               │ │
│  │ btrfs快照   │  │ 硬件看门狗   │  │ TPM           │ │
│  │ 操作前备份  │  │ 软件心跳     │  │ Secure Boot   │ │
│  │ 失败恢复    │  │ 异常重启     │  │ 密钥管理       │ │
│  └─────────────┘  └──────────────┘  └────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

---

## 9. 认知引擎设计

### 9.1 推理架构

```
┌─────────────────────────────────────────────────────────┐
│                    认知引擎                               │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │              推理循环 (Think-Act-Observe)          │  │
│  │                                                   │  │
│  │  ┌──────────┐   ┌──────────┐   ┌──────────┐     │  │
│  │  │ THINK    │──▶│ PLAN     │──▶│ ACT      │     │  │
│  │  │          │   │          │   │          │     │  │
│  │  │ 分析当前  │   │ 制定计划  │   │ 执行动作  │     │  │
│  │  │ 状态和   │   │ 分解步骤  │   │ 调用工具  │     │  │
│  │  │ 目标     │   │ 选择策略  │   │ 观察结果  │     │  │
│  │  └──────────┘   └──────────┘   └──────────┘     │  │
│  │       ▲                                  │        │  │
│  │       │                                  │        │  │
│  │       └──────────────────────────────────┘        │  │
│  │                   反馈循环                          │  │
│  └───────────────────────────────────────────────────┘  │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │              推理模式                               │  │
│  │                                                   │  │
│  │  ReAct:      推理→行动→观察→推理→...              │  │
│  │  Plan&Exec:  先规划全部步骤，再逐步执行            │  │
│  │  Reflexion:  执行后反思，改进下次行为              │  │
│  │  TreeSearch:  探索多条路径，选择最优               │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### 9.2 工具系统

```yaml
# Agent 可以使用的工具 (Tool/Function Calling)
tools:
  system:
    - shell_exec         # 执行 shell 命令
    - file_read          # 读取文件
    - file_write         # 写入文件
    - file_search        # 搜索文件 (find/rg)
    - process_list       # 列出进程
    - process_kill       # 终止进程
    - service_control    # systemd 服务控制
    - network_info       # 网络状态查询
    - network_config     # 网络配置修改

  perception:
    - screen_capture     # 屏幕截图 + OCR
    - audio_record       # 录音 + STT
    - sensor_read        # 读取传感器数据
    - camera_capture     # 摄像头拍照
    - notification_read  # 读取通知
    - clipboard_read     # 读取剪贴板

  communication:
    - send_notification  # 发送通知
    - send_message       # 发送消息 (各种平台)
    - send_email         # 发送邮件
    - make_http          # HTTP 请求
    - mqtt_publish       # MQTT 发布

  reasoning:
    - code_interpreter   # 执行 Python 代码
    - web_search         # 网络搜索
    - calculator         # 计算器
    - knowledge_query    # 知识库查询

  control:
    - mouse_click        # 模拟鼠标点击
    - keyboard_type      # 模拟键盘输入
    - app_launch         # 启动应用
    - gpio_control       # GPIO 控制 (嵌入式)
    - servo_control      # 电机控制 (嵌入式)
```

---

## 10. 记忆系统

### 10.1 分层记忆架构

```
┌─────────────────────────────────────────────────────────┐
│                    记忆系统                               │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │  L1: 工作记忆 (Working Memory)                     │  │
│  │  ├── 当前对话上下文                                 │  │
│  │  ├── 当前任务状态                                   │  │
│  │  ├── 最近感知数据                                   │  │
│  │  ├── 存储: RAM                                      │  │
│  │  └── 容量: 有限 (上下文窗口)                         │  │
│  └───────────────────────────────────────────────────┘  │
│                         │ 定期压缩                        │
│                         ▼                                │
│  ┌───────────────────────────────────────────────────┐  │
│  │  L2: 短期记忆 (Short-term Memory)                   │  │
│  │  ├── 最近 24-48 小时的事件                          │  │
│  │  ├── 用户最近的偏好和行为                            │  │
│  │  ├── 当前项目的上下文                                │  │
│  │  ├── 存储: SQLite                                   │  │
│  │  └── 容量: GB 级                                    │  │
│  └───────────────────────────────────────────────────┘  │
│                         │ 定期整理                        │
│                         ▼                                │
│  ┌───────────────────────────────────────────────────┐  │
│  │  L3: 长期记忆 (Long-term Memory)                    │  │
│  │  ├── 用户习惯和模式                                  │  │
│  │  ├── 历史决策和结果                                  │  │
│  │  ├── 学到的知识                                     │  │
│  │  ├── 存储: 向量数据库 (ChromaDB/Qdrant)             │  │
│  │  └── 容量: TB 级                                    │  │
│  └───────────────────────────────────────────────────┘  │
│                         │ 跨设备同步 (可选)               │
│                         ▼                                │
│  ┌───────────────────────────────────────────────────┐  │
│  │  L4: 共享记忆 (Shared Memory) [可选]                │  │
│  │  ├── 多设备间同步的记忆                              │  │
│  │  ├── 团队/家庭共享的知识                             │  │
│  │  ├── 存储: 云端/本地 NAS                             │  │
│  │  └── 同步: 端到端加密                                │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### 10.2 记忆操作

```
记忆写入:
├── 事件记忆 — "发生了什么"
├── 事实记忆 — "用户说了什么"
├── 程序记忆 — "怎么做某件事"
└── 情景记忆 — "在什么场景下做了什么"

记忆检索:
├── 时间索引 — "昨天发生了什么"
├── 语义搜索 — "关于项目 X 的事情"
├── 关联检索 — "和这件事相关的历史"
└── 模式匹配 — "上次类似的情况是怎么处理的"

记忆整理:
├── 压缩 — 短期记忆 → 长期记忆的摘要
├── 遗忘 — 不重要的细节逐渐淡忘
├── 强化 — 被反复访问的记忆变得更强
└── 关联 — 建立记忆之间的联系
```

---

## 11. 感知层设计

### 11.1 多模态感知融合

```
┌─────────────────────────────────────────────────────────┐
│                    感知层                                 │
│                                                         │
│  系统感知:                                               │
│  ├── eBPF — 内核事件 (进程/文件/网络)                    │
│  ├── /proc — 进程状态                                   │
│  ├── /sys — 硬件状态                                    │
│  ├── journald — 系统日志                                │
│  ├── inotify — 文件系统变化                              │
│  └── udev — 设备热插拔                                  │
│                                                         │
│  用户感知:                                               │
│  ├── 屏幕 OCR — 当前屏幕内容                             │
│  ├── 键盘/鼠标 — 用户输入模式                            │
│  ├── 剪贴板 — 复制粘贴内容                               │
│  ├── 应用状态 — 当前在用什么                              │
│  └── 通知流 — 收到什么通知                               │
│                                                         │
│  环境感知:                                               │
│  ├── 摄像头 — 视觉输入                                   │
│  ├── 麦克风 — 音频输入                                   │
│  ├── 传感器 — 温度/湿度/光照/运动                        │
│  ├── GPS/网络定位 — 位置                                 │
│  └── 时间/日历 — 时间上下文                              │
│                                                         │
│  网络感知:                                               │
│  ├── DNS 查询 — 域名解析                                 │
│  ├── HTTP 流量 — 网络请求 (可选, 需授权)                 │
│  ├── RSS/Feed — 信息源订阅                               │
│  └── 消息流 — 各平台消息                                 │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### 11.2 感知事件模型

```json
// 感知事件统一格式
{
  "event_id": "evt_20260606_001",
  "timestamp": "2026-06-06T14:30:00+08:00",
  "source": "ebpf/file_access",
  "priority": "normal",
  "data": {
    "pid": 12345,
    "comm": "code",
    "action": "open",
    "path": "/home/user/project/src/main.rs"
  },
  "context": {
    "user_focus": "coding",
    "current_project": "argos",
    "time_of_day": "afternoon"
  }
}
```

---

## 12. 执行层设计

### 12.1 执行引擎架构

```
┌─────────────────────────────────────────────────────────┐
│                    执行层                                 │
│                                                         │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ 命令执行器    │  │ API 调用器    │  │ 硬件控制器    │  │
│  │              │  │              │  │              │  │
│  │ Shell exec   │  │ HTTP client  │  │ GPIO         │  │
│  │ Python exec  │  │ D-Bus call   │  │ I2C/SPI      │  │
│  │ Node exec    │  │ gRPC call    │  │ PWM          │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ 系统管理器    │  │ UI 控制器     │  │ 通信控制器    │  │
│  │              │  │              │  │              │  │
│  │ systemd      │  │ 鼠标/键盘    │  │ MQTT         │  │
│  │ 包管理       │  │ 窗口管理     │  │ HTTP Server  │  │
│  │ 用户管理     │  │ 通知         │  │ WebSocket    │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │              执行沙箱 (每个执行都在沙箱中)          │  │
│  │                                                   │  │
│  │  bubblewrap namespace → 文件系统隔离               │  │
│  │  cgroups → 资源限制                                │  │
│  │  seccomp → 系统调用过滤                            │  │
│  │  netns → 网络隔离                                  │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### 12.2 执行结果模型

```json
{
  "action_id": "act_20260606_001",
  "tool": "shell_exec",
  "command": "make build",
  "exit_code": 0,
  "stdout": "...",
  "stderr": "",
  "duration_ms": 3420,
  "side_effects": [
    {"type": "file_created", "path": "/build/output"},
    {"type": "process_spawned", "pid": 12346}
  ],
  "rollback_info": {
    "snapshot_id": "snap_001",
    "can_rollback": true
  }
}
```

---

## 13. 混合推理架构 (本地+云端)

### 13.1 推理决策树

```
用户请求 / 系统事件
        │
        ▼
┌───────────────────┐
│ 意图分类          │ ← 本地小模型 (1B, <10ms)
│ 简单/中等/复杂    │
└────────┬──────────┘
         │
    ┌────┴────────────────────┐
    │                         │
    ▼                         ▼
┌──────────┐           ┌──────────────┐
│ 本地推理  │           │ 云端推理      │
│          │           │              │
│ llama.cpp│           │ Claude/GPT   │
│ Qwen3-8B │           │ DeepSeek     │
│ Q4 量化   │           │ MiMo         │
│          │           │              │
│ 适合:     │           │ 适合:        │
│ 日常任务  │           │ 复杂推理      │
│ 简单问答  │           │ 代码生成      │
│ 系统控制  │           │ 数学计算      │
│ 模式匹配  │           │ 长文分析      │
│          │           │              │
│ 延迟:<1s │           │ 延迟:1-10s   │
│ 隐私:✓   │           │ 隐私:需授权  │
│ 离线:✓   │           │ 离线:✗       │
└──────────┘           └──────────────┘
```

### 13.2 云端 API 管理

```yaml
# /etc/agent/providers.yaml
providers:
  local:
    engine: llama-cpp
    model: /var/lib/agent/models/qwen3-8b-q4.gguf
    context_length: 8192
    gpu_layers: 99  # 全部放 GPU

  cloud:
    - name: openai
      base_url: https://api.openai.com/v1
      api_key: ${OPENAI_API_KEY}
      models: [gpt-4o, gpt-4o-mini]
      priority: 2

    - name: deepseek
      base_url: https://api.deepseek.com/v1
      api_key: ${DEEPSEEK_API_KEY}
      models: [deepseek-chat, deepseek-reasoner]
      priority: 1

    - name: anthropic
      base_url: https://api.anthropic.com/v1
      api_key: ${ANTHROPIC_API_KEY}
      models: [claude-sonnet-4-20250514]
      priority: 3

  routing:
    default: local
    fallback: cloud
    escalation_rules:
      - condition: "token_count > 4096"
        target: cloud
      - condition: "task_type == 'code_generation'"
        target: cloud
      - condition: "confidence < 0.7"
        target: cloud
      - condition: "user_explicitly_requests"
        target: cloud
```

---

## 14. 实现路线图

### Phase 1: 最小可用 Agent (1-2周)

```
目标: 能在 Arch Linux 上跑起来的 agentd

├── [ ] 项目骨架搭建
│   ├── [ ] Rust 项目初始化 (Cargo workspace)
│   ├── [ ] 目录结构
│   └── [ ] CI/CD 基础
│
├── [ ] 核心引擎
│   ├── [ ] 推理循环 (Think-Act-Observe)
│   ├── [ ] 本地 llama.cpp 集成
│   ├── [ ] Shell 执行引擎
│   └── [ ] 基础工具注册
│
├── [ ] 系统服务
│   ├── [ ] systemd service 定义
│   ├── [ ] 配置文件解析
│   ├── [ ] 日志系统
│   └── [ ] 看门狗
│
├── [ ] 交互层
│   ├── [ ] CLI 客户端 (agent-cli)
│   └── [ ] Socket 通信
│
└── [ ] 基础安全
    ├── [ ] 权限分级策略
    ├── [ ] 命令白名单/黑名单
    └── [ ] 审计日志
```

### Phase 2: 系统感知 (2-4周)

```
目标: Agent 能"看见"系统

├── [ ] 感知层
│   ├── [ ] inotify 文件监控
│   ├── [ ] /proc /sys 轮询
│   ├── [ ] journald 日志流
│   ├── [ ] 网络状态监控
│   └── [ ] eBPF 感知模块 (进阶)
│
├── [ ] 记忆系统
│   ├── [ ] SQLite 事件存储
│   ├── [ ] 向量检索 (ChromaDB)
│   ├── [ ] 记忆压缩/整理
│   └── [ ] 上下文管理
│
├── [ ] 扩展工具
│   ├── [ ] 屏幕截图 + OCR
│   ├── [ ] 剪贴板监听
│   └── [ ] 系统信息查询
│
└── [ ] D-Bus 接口
    ├── [ ] Agent D-Bus 服务
    └── [ ] 其他程序调用接口
```

### Phase 3: 深度控制 (1-2月)

```
目标: Agent 能"控制系统"

├── [ ] systemd 集成
│   ├── [ ] 服务管理 (启停/重启)
│   ├── [ ] cgroups 资源控制
│   ├── [ ] timer 管理
│   └── [ ] journal 查询
│
├── [ ] 包管理集成
│   ├── [ ] pacman 查询/安装/更新
│   ├── [ ] AUR 支持 (yay)
│   └── [ ] 依赖分析
│
├── [ ] 网络控制
│   ├── [ ] NetworkManager D-Bus
│   ├── [ ] iptables 规则管理
│   └── [ ] DNS 配置
│
├── [ ] FUSE 虚拟文件系统
│   ├── [ ] /mnt/agent 挂载点
│   ├── [ ] 上下文暴露
│   ├── [ ] 控制接口
│   └── [ ] 传感器数据
│
├── [ ] 云端推理 Fallback
│   ├── [ ] 多 Provider 支持
│   ├── [ ] 智能路由
│   └── [ ] API Key 管理
│
└── [ ] 任务规划引擎
    ├── [ ] ReAct 模式
    ├── [ ] Plan & Execute 模式
    └── [ ] 反思/改进
```

### Phase 4: Android 支持 (1-2月)

```
目标: Agent 在 Android 上运行

├── [ ] Android App 框架
│   ├── [ ] Foreground Service
│   ├── [ ] JNI 桥接 (Agent Core)
│   └── [ ] UI (Jetpack Compose)
│
├── [ ] Android 感知层
│   ├── [ ] AccessibilityService
│   ├── [ ] NotificationListener
│   ├── [ ] UsageStats
│   └── [ ] 传感器接入
│
├── [ ] Android 执行层
│   ├── [ ] Intent 系统
│   ├── [ ] UI 自动化
│   ├── [ ] 通知管理
│   └── [ ] 应用管理
│
├── [ ] Root 扩展 (可选)
│   ├── [ ] Shell 执行
│   ├── [ ] 系统属性
│   └── [ ] 网络控制
│
└── [ ] 跨设备同步
    ├── [ ] 记忆同步
    └── [ ] 任务同步
```

### Phase 5: 嵌入式 SDK (1-2月)

```
目标: Agent 可以跑在开发板上

├── [ ] C SDK
│   ├── [ ] 核心库 (libagent.a/.so)
│   ├── [ ] 感知接口 API
│   ├── [ ] 执行接口 API
│   └── [ ] 记忆接口 API
│
├── [ ] 硬件适配
│   ├── [ ] RK3588 BSP
│   ├── [ ] GPIO/I2C/SPI 驱动
│   ├── [ ] NPU 推理加速
│   └── [ ] 摄像头/麦克风接入
│
├── [ ] 轻量推理
│   ├── [ ] Qwen3-3B GGUF 量化
│   ├── [ ] NPU delegate
│   └── [ ] 推理优化
│
├── [ ] 通信层
│   ├── [ ] MQTT
│   ├── [ ] HTTP Server
│   ├── [ ] BLE
│   └── [ ] WiFi 管理
│
└── [ ] 示例项目
    ├── [ ] 智能家居控制器
    ├── [ ] 边缘监控系统
    └── [ ] 机器人控制
```

### Phase 6: 智能进化 (持续)

```
目标: Agent 越用越聪明

├── [ ] 行为学习
│   ├── [ ] 用户习惯识别
│   ├── [ ] 自动化工作流生成
│   ├── [ ] 异常模式检测
│   └── [ ] 个性化推荐
│
├── [ ] 多 Agent 协作
│   ├── [ ] Agent 间通信协议
│   ├── [ ] 任务分发
│   ├── [ ] 知识共享
│   └── [ ] 冲突解决
│
├── [ ] 生态建设
│   ├── [ ] 插件系统
│   ├── [ ] 技能市场
│   ├── [ ] 社区贡献
│   └── [ ] 文档和教程
│
└── [ ] 前沿探索
    ├── [ ] 情感计算
    ├── [ ] 主动学习
    ├── [ ] 自我改进
    └── [ ] 多模态融合
```

---

## 15. 技术选型

### 核心技术栈

| 层次 | 技术 | 选型理由 |
|------|------|----------|
| **核心语言** | Rust | 安全、高性能、系统级、跨平台 |
| **脚本/插件** | Python | 生态丰富、快速开发 |
| **本地推理** | llama.cpp | 轻量、跨平台、社区活跃 |
| **语音** | whisper.cpp | 离线语音识别 |
| **向量存储** | ChromaDB / Qdrant | 本地向量数据库 |
| **关系存储** | SQLite | 嵌入式、零配置 |
| **配置** | TOML + YAML | 可读性好 |
| **日志** | tracing (Rust) | 结构化日志 |
| **进程间通信** | Unix Socket + D-Bus (Linux) | 低延迟、系统级 |
| **IPC (Android)** | Binder / AIDL | Android 原生 |
| **IPC (嵌入式)** | MQTT + HTTP | 轻量、广泛支持 |
| **沙箱** | bubblewrap + seccomp | 轻量级隔离 |
| **安全策略** | OPA / Casbin | 声明式策略 |
| **构建** | Cargo + CMake (SDK) | Rust 生态 + C 交叉编译 |
| **包管理** | Cargo (Rust) + AUR (Arch) | 系统集成 |

### 开发板 SDK 技术栈

| 组件 | 技术 | 说明 |
|------|------|------|
| 核心库 | C / Rust | 静态链接，嵌入式友好 |
| 绑定 | Python / Rust / MicroPython | 多语言支持 |
| 推理 | llama.cpp + RKNN (RK3588) | 本地 + NPU 加速 |
| 通信 | libmosquitto (MQTT) | 轻量消息队列 |
| 存储 | SQLite + 小型 KV | Flash 友好 |
| 固件 | Buildroot / Yocto | 嵌入式 Linux 构建 |

---

## 16. 开放问题

### 待研究

```
1. Agent 的"自我意识"边界在哪里?
   ├── Agent 应该有多大的自主权?
   ├── 什么时候应该停下来问人?
   └── 如何避免 Agent "过度自信"?

2. 隐私和安全的平衡
   ├── Agent 需要看到多少用户数据才能有效工作?
   ├── 本地处理 vs 云端处理的边界
   └── 多设备同步时的数据保护

3. 记忆的"遗忘"策略
   ├── 什么该记住，什么该忘记?
   ├── 如何避免记忆污染?
   └── 跨设备记忆冲突解决

4. 多 Agent 协作
   ├── 多个 Agent 如何分工?
   ├── 冲突如何解决?
   └── 共享知识的粒度

5. 法律和伦理
   ├── Agent 做出的决定谁负责?
   ├── Agent 的行为日志的法律效力
   └── 不同司法管辖区的要求

6. 性能和资源
   ├── 本地推理的质量和速度平衡
   ├── 记忆系统的存储效率
   └── 感知层的 CPU/内存开销

7. Android 碎片化
   ├── 不同厂商的权限差异
   ├── 后台保活策略
   └── 无 Root 情况下的能力边界
```

---

## 附录

### A. 参考项目

| 项目 | 相关性 | 链接 |
|------|--------|------|
| Open Interpreter | 系统控制 Agent | github.com/OpenInterpreter |
| Aider | 代码 Agent | github.com/paul-gauthier/aider |
| llama.cpp | 本地推理引擎 | github.com/ggerganov/llama.cpp |
| whisper.cpp | 本地语音识别 | github.com/ggerganov/whisper.cpp |
| Ollama | 本地模型管理 | github.com/ollama/ollama |
| bubblewrap | 轻量沙箱 | github.com/containers/bubblewrap |
| aurb | Agent 技能框架 | 本项目 |

### B. 术语表

| 术语 | 含义 |
|------|------|
| Agent Runtime | Agent 的核心运行时环境 |
| eBPF | Extended Berkeley Packet Filter, Linux 内核可编程框架 |
| FUSE | Filesystem in Userspace, 用户态文件系统 |
| systemd | Linux 系统和服务管理器 |
| D-Bus | Desktop Bus, Linux 桌面消息总线 |
| NPU | Neural Processing Unit, 神经网络处理单元 |
| GGUF | GPT-Generated Unified Format, 模型量化格式 |
| ReAct | Reasoning + Acting, 推理行动框架 |
| bubblewrap | 轻量级 Linux 沙箱工具 |
| seccomp | Secure Computing Mode, 系统调用过滤 |
| cgroups | Control Groups, Linux 资源控制 |
| Binder | Android IPC 机制 |
| AccessibilityService | Android 无障碍服务, 可感知屏幕内容 |

---

*文档版本: 0.1.0*
*最后更新: 2026-06-06*
