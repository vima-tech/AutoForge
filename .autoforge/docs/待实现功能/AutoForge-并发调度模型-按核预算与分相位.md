# AutoForge 并发调度模型：按核预算 + 分相位

> 你限的是**逻辑单元**（agent），抢的是**物理核**；推理外包给了远端，本地留下的全是**验证的算力债**。
> 瓶颈不在 agent 多少，在于并发模型没把「等待相」和「计算相」分开记账。

---

## 0. 这份文档要解决的一件事

把 admission control 从「数 agent」迁移到「按核预算 + 分相位租约」，用**同一个机制**同时拿到三个收益：

1. 无论放行多少 agent，都封住 CPU 过度订阅（oversubscription）；
2. 把同时撞上的验证尖峰自动错开（消除 CPU 上的 thundering herd）；
3. 不用测量、不用调参，自动逼近最优并发数 `核数 × (1 + W/C)`。

一个机制、三个payoff——这是本设计的脊椎。其余全是它的兜底和接线。

---

## 1. 现象归因：为什么 CPU 满、内存平

前提：**`claude -p` 是 I/O-bound 的编排进程，不是 CPU-bound 的推理进程。** 推理在 Anthropic 远端，大部分 wall-clock 花在等流式响应，那段时间本地核是空的。

本地 CPU 的真正消费者是 agent 触发的三类本地计算：

| 消费者 | 类型 | 内存特征 | 说明 |
|---|---|---|---|
| 验证循环（tsc/mypy/eslint/ruff/pytest/build） | 突发、纯算力 | working set 只跟代码库走，不随负载涨 | **主力**。编译和测试都是 CPU-bound |
| preview 层 dev server + 文件监听 | 常驻低 CPU 地板 | 极低 | overlayfs 下 inotify 常失效，chokidar 退化成 `usePolling` → 纯 CPU 空转 |
| stream-json 解析 | 稳定小量 | 低 | N 个 Node event loop 各自解流 |

**内存不涨是最强诊断信号**：它排除了内存泄漏、排除了本地加载大模型/大数据集。你看到的是**计算密集、非数据密集**的负载——那个「大」的东西（模型）被卸载到远端了，本地只剩瞬时算完即弃的验证。

---

## 2. 为什么静态的「5」在原理上一定是错的

### 2.1 粒度错配

Redis 信号量**限的是 agent（逻辑单元），争的是核（物理单元）**，两者不成比例。每个放行的 agent 会 fan-out 出不受控的子进程：pytest 并行 worker、并行 typecheck、构建。

```
真实并行度 = agent 数 × 子进程扇出
5 agent × 每个 4 个 pytest worker = 20 个 CPU-hungry 进程抢 8 核 → 过度订阅 → 上下文切换空转
```

这个乘积从未对齐过核数——信号量对「一个 agent 放行后到底占几个核」一无所知。

### 2.2 相位不协调（thundering herd）

单个 agent 的 CPU 占用是**突发**的：等 API 时空闲，进验证时尖峰。5 个 agent 的验证尖峰若**同时**撞上，就是一次 CPU 上的惊群；若错开，核又闲着。静态信号量对相位无感知。

### 2.3 最优并发数是个变量，不是常数

混合 I/O + CPU 负载的最优并发：

```
最优 agent 数 ≈ 核数 × (1 + W/C)
    W = 等 API 的时间（I/O 相）
    C = 本地验证的 CPU 时间（计算相）
```

**W/C 每个任务都不同**：
- 带巨型测试套件的重构 → W/C 低 → 几个 agent 就饱和；
- 改一行的小编辑 → W/C 高 → 能塞很多。

所以任何静态常数（「5」）都假设了一个不存在的常量。越过 CPU 饱和点后继续加 agent，吞吐**不升反降**（切换开销吃掉收益）。

---

## 3. 核心洞察：相位分离 + 按核预算

一个 agent run 不是一坨匀质负载，而是一串相位。只有**计算相**该抢核预算，其余相位免费并发。

| 相位 | 主导资源 | 是否占核预算 | 时长特征 |
|---|---|---|---|
| PLAN / ANALYZE | API 等待（I/O） | 否 | 长、廉价 |
| EDIT | API 等待 + 极小本地写 | 否 | 中 |
| **VERIFY**（typecheck/lint/test/build） | **CPU** | **是（占 = 权重）** | 短、尖峰 |
| PREVIEW（dev server + watcher） | 常驻低 CPU | 小额固定扣减 | 长、恒定地板 |
| REVIEW WAIT（等人） | 无 | 否 | 不定、全空 |

**关键**：agent 不该在整个生命周期持有 CPU 令牌，只在进入计算相时短暂持有。这是**相位作用域的租约（phase-scoped lease）**。

---

## 4. 模型设计：两层准入

### 4.1 双层结构

- **Tier 1 · 会话槽位（session slot）**——宽松。约束的是内存 / fd / API 并发这类「按 agent 计」的东西，**不约束 CPU**。默认给到 `核数 × 2~3`，目的是让 I/O-wait 的任务把管道填满。
- **Tier 2 · 核预算信号量（CPU budget）**——以**核**为单位计数，只在验证/构建步骤前后加锁。这才是真正的限流器。

会话槽位从头持有到尾；核令牌只在计算相租借。

### 4.2 加权：每个计算步声明权重

权重 = 该步骤预期的并行度：

```
tsc          → 1
eslint/ruff  → 1
mypy         → 2
pytest -n K  → K（建议封顶，如 4）
cargo build  → 4
vite build   → 4
```

### 4.3 相位作用域租约协议

```
acquire session_slot                 # Tier 1，整个 run 持有
    run claude -p                    # I/O 等待，不持核令牌
    on enter VERIFY:
        acquire cpu_budget(weight)   # Tier 2，预算不足则排队阻塞
        run verify，并把子进程扇出封顶到 weight
        release cpu_budget(weight)
    ...
release session_slot
```

### 4.4 为什么它**自动**实现 `核数 × (1+W/C)`——本设计最漂亮的地方

你**不需要测 W/C，也不需要调参**。核预算信号量会让公式自我兑现：

- 高 W/C 的任务（小编辑）→ 只在很短的时间里持有核令牌 → 同一时刻很多个能共存；
- 低 W/C 的任务（大重构）→ 长时间攥着核令牌 → 同一时刻只有少数能共存。

于是「同时活跃的 agent 数」**涌现**为 `核数 × (1+W/C)`，不用任何人去计算 W 和 C。会话槽位放得宽，核预算做真限制——两者一配合，最优并发自动成立。

这就是「一个机制复用到底」：**同一个核预算信号量，既封了过度订阅（§2.1），又把惊群排成流水线（§2.2），还自执行了最优并发公式（§2.3）**。

---

## 5. 落地机制

### 5.1 子进程扇出必须封顶（否则记账在撒谎）

声明了 weight=4，就必须真把子进程压到 4，否则信号量算的是假账：

```
pytest -n {weight}
MAKEFLAGS=-j{weight}    /    cargo build -j {weight}
tsc 视为单线程（或按其真实线程计权重）
其余工具显式传并行度参数
```

### 5.2 cgroup 硬兜底（信号量是协作式的，需要强制层）

信号量依赖「权重声明正确」，是君子协定。加一层硬约束兜住撒谎的权重和失控的子进程树：

- v2 cgroup：把所有 verify 子进程放进一个父 cgroup，`cpu.max = <核数>`；
- 或每个 agent 沙箱 `--cpus=<weight>`（你已经有 Docker/Podman 隔离，天然具备）。

权重低估 → 过度订阅回潮 → cgroup 兜住 + 从实测线程数回填修正权重。

### 5.3 preview watcher 地板治理（这是配置问题，别为它改调度器）

先修根因：**强制 inotify，别让它退化成 polling**。退化不可避免时：
- 把它计入账：`可用核 = 核数 − N_preview × 单个 poll 成本`；
- 更好：**没有活跃 reviewer 的 preview 挂起**（暂停 dev server，或降级为仅文件事件、停轮询）。

判据：如果烧核的是 watcher 轮询，那是配置问题；如果是 verify 子进程突发，才是该动的架构问题。动手前先用 `pidstat` / cgroup stats / `py-spy` 确认到底是谁在烧核。

---

## 6. 可观测性与收敛信号

**每相位记账**：wall time、CPU-time、持有的令牌数。
**全局**：核预算利用率、CPU 信号量队列深度、load average vs 核数。

调对了的信号长这样：

- 稳态下 `load average ≈ 核数`（既不空转也不过载）；
- CPU 信号量队列**非空但有界**（说明限流在起作用，且没堆积）；
- 除了瞬时抖动，几乎没有 cgroup throttling 事件。

这三条同时成立 = 相位分离与核预算参数正确。

---

## 7. 边界与失败模式

| 情况 | 处理 |
|---|---|
| 权重低估 → 过度订阅回潮 | cgroup 兜底 + 从实测线程数回填权重 |
| 单个 verify 权重 > 总核预算 | 允许，但把 permits 夹到「核数」，它独占整机运行；**绝不死锁** |
| 加权信号量的大权重饥饿 | acquire N 用 Lua 原子 check-and-decrement，配 FIFO 公平队列，防止大权重被小权重永久插队 |
| 优先级反转（小快 verify 卡在大 verify 后） | 可选：分「轻 verify 通道 / 重 verify 通道」双 lane；不咬人就先不做 |
| REVIEW WAIT 长期占会话槽位 | 会话槽位本就不该是稀缺资源（核才是）；若日后 fd/内存成墙，再单独 cap「并发活跃项目数」 |

---

## 8. 参数化示例（8 可用核）

```
可用核             = 8                （nproc 减去留给控制平面的余量）
session slot       = 16               （宽松，让 wait-bound 任务填满管道）
cpu_budget         = 8 permits        （oversubscribe 1.0；verify 含大量读文件 I/O 时可上调到 10）
verify 父 cgroup   = cpu.max 800000 100000   （硬顶 8 核）

权重表：
  tsc / eslint / ruff = 1
  mypy                = 2
  pytest              = 4（封顶）
  cargo build         = 4
  vite build          = 4

preview：强制 inotify；无 reviewer 空闲 N 分钟后挂起
```

场景演算：5 个 agent 同时进 VERIFY，各权重 4，预算 8 → 只有 2 个并发跑，其余 3 个排队。**队列本身就是相位错开机制**，把同时惊群转成流水线；而它们的 API-wait 相从未碰过预算，wait-bound 部分的吞吐照样高。

---

## 9. 伪代码：Redis 加权信号量 + 相位租约

```python
# ---- Redis 加权信号量：原子多令牌获取（Lua）----
ACQUIRE_LUA = """
local key   = KEYS[1]
local avail = tonumber(redis.call('GET', key) or ARGV[2])
local need  = tonumber(ARGV[1])
-- 权重超过总预算时，夹到总预算，允许独占，绝不死锁
if need > tonumber(ARGV[2]) then need = tonumber(ARGV[2]) end
if avail >= need then
    redis.call('DECRBY', key, need)
    return need            -- 返回实际扣减，用于 release
end
return -1                  -- 预算不足，调用方进 FIFO 队列等待唤醒
"""

class CoreBudget:
    def __init__(self, redis, total_cores):
        self.r, self.key, self.total = redis, "autoforge:cpu_budget", total_cores
        self.r.set(self.key, total_cores)

    async def acquire(self, weight):
        while True:
            got = self.r.eval(ACQUIRE_LUA, 1, self.key, weight, self.total)
            if got != -1:
                return got                      # 实际持有的令牌数
            await self._wait_fifo()             # 公平排队，防大权重饥饿

    def release(self, granted):
        self.r.incrby(self.key, granted)
        self._notify_fifo()

# ---- 相位租约：只在计算相持核令牌 ----
async def run_agent(task, slots: SessionSlots, budget: CoreBudget):
    async with slots.acquire():                 # Tier 1，全程持有，不占核
        await claude_p_analyze(task)            # I/O 相，免费并发
        await claude_p_edit(task)               # I/O 相
        w = weight_of(task.verify_step)         # 声明权重
        granted = await budget.acquire(w)       # Tier 2，进计算相才抢核
        try:
            await run_verify(task, parallelism=granted,   # 子进程扇出封顶
                             cgroup=f"verify/{task.id}")  # cgroup 硬兜底
        finally:
            budget.release(granted)             # 出计算相立刻归还
        await start_preview(task)               # 常驻低 CPU 地板，另计
        await wait_for_review(task)             # 等人，全空
```

---

## 10. 与 AutoForge 现有架构的接线

- **背压三阶段（正常/降速/暂停）**：现在是全局的；改成**以核预算队列深度为信号**。队列 > 阈值 → 降速（新 agent 只放行到 wait 相，卡在 verify 前）；throttling 事件 > 阈值 → 暂停放行。背压的观测量从「agent 数」换成「核预算压力」。
- **并发槽位默认 5**：拆成 `session slot（宽）+ cpu budget（真限）` 两个数。原来的「5」退役——它把两件事焊死了。
- **成本核算（阶段三再深做）**：相位记账（每相 wall/CPU-time）天然就是成本核算的原始账本，现在先埋点，商业化时直接复用，不用二次改造。
- **in-system preview**：preview 的 CPU 地板纳入核预算的固定扣减项；空闲 preview 挂起 = 把核还给 verify。preview 与 verify 争的是同一池核，必须统一记账。

---

## 11. 一页纸决策清单

- [ ] 先归因：`pidstat`/`py-spy`/cgroup stats 确认烧核的是 watcher 轮询还是 verify 突发
- [ ] watcher：强制 inotify；空闲 preview 挂起（**这是配置修复，非架构改动**）
- [ ] 拆分并发控制：session slot（核数×2~3）+ cpu budget（≈核数）
- [ ] 给每个 verify 步声明权重，并把子进程扇出**真封顶**到权重
- [ ] cgroup / `--cpus` 做硬兜底，防权重撒谎
- [ ] 相位租约：只在 VERIFY 前后借还核令牌，其余相免费并发
- [ ] Lua 原子多令牌 + FIFO 公平队列，防大权重饥饿；权重>预算时夹到预算独占，不死锁
- [ ] 埋点：每相位 wall/CPU-time + 队列深度 + load average
- [ ] 收敛判据：load ≈ 核数、队列非空但有界、几乎无 throttling
- [ ] 背压信号切到「核预算队列深度」；退役静态「5」

---

**收口**：让 agent 数放宽、核预算做真限、只在计算相记账——同一个信号量把过度订阅、惊群、最优并发一次解决。推理外包了，本地这台机器只需管好自己那笔算力债。
