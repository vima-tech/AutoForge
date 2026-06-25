# BRANCHING.md — AutoForge 分支管理规范

> 本文件是 AutoForge 仓库的**分支协作契约**。当 `dev` 分支同时被多方写入时
> ——**AutoForge 自身**(自管理合并)、**同事**、**你本人的多个并行 Claude session**
> ——本规范保证:并行任务零干扰、随时吸收他人推进、落地前强校验、最终落地原子且永不"双头"。
>
> 适用对象:在本仓库做人工开发的工程师(含驱动多个 Claude Code session 的场景)。
> AutoForge 产品内部的编码/合并链路无需改动,本规范与其"先对齐再落地"哲学天然兼容。
>
> **本规范已自动托管**(见 §8):`.claude/settings.json` 的 hooks 会引导每个 Claude
> session 并**硬拦截**违规 git/编辑操作。脚本全文见 §9 附录。

---

## 0. 背景:为什么需要这份规范

AutoForge 是自托管的——它会改自己这个仓库。其**自管理合并路径**的实际行为(见
`src-tauri/src/tasks/execution.rs:118`、`src-tauri/src/tasks/merge.rs:15`):

1. **编码阶段**:每个 CR `git fetch origin <dev>`,worktree **以 `origin/<dev>` 为基点**创建
   → AutoForge 永远基于"远程 dev 最新",看不到你本地 dev 上未推送的改动。
2. **合并阶段**:用 `git branch --show-current` 判断 `dev_is_live`(主工作树当前是否签出 `dev`)。
   - `dev_is_live == true`(自管理场景):在一次性 worktree 里基于 `origin/<dev>` 合并,
     然后 **`git push origin HEAD:<dev>`**,**完全不碰你本地工作树**。
   - `dev_is_live == false`:走普通项目快路径,在**主工作树**里 `checkout dev && merge --squash && commit`
     (本地提交、**不 push**)——会把主仓从你的分支拽到 dev、可能因未提交改动而失败。

**结论**:AutoForge 本质是一个 **纯 `origin/dev` 写入者**;本地 `dev` 何时前进**完全由你掌控**。
若你也在**本地 `dev`** 上提交,就会与 `origin/dev` 形成两个独立 head(双头)→ push 被拒、pull 冲突。
再叠加多个并行 session,问题指数放大。

**本规范能消除的 vs 不能消除的**:它根治**集成冲突 / 工作树互踩 / 双头**;它**不能**自动消除
"两个 session 同时改同一个核心文件/同一个 migration/同一个 lockfile"这类**编辑冲突**——
后者需要 §7 的「冲突域控制规则」配合。两者合起来才接近"最大限度降低代码冲突"。

---

## 1. 核心原则(必读)

1. **`origin/dev` 是唯一集成主干**。所有写入者(AutoForge / 同事 / 你的每个 session)落地前
   都必须先对齐它,再 push。
2. **主仓 `dev` 是只读镜像**:保持签出 `dev` 且工作树**永远干净**,谁都不在主仓里直接动手,
   只用 `git pull --ff-only` 追新。
   - 保持签出 dev → AutoForge 的 `dev_is_live=true` 持续成立,走安全 push 路径,不抢你工作树;
   - 本地 dev 从不被你写 → 对 `origin/dev` 永远是**纯快进**,`pull --ff-only` 永不冲突。
3. **一任务 = 一 worktree = 一 feature 分支 = 一个 Claude session**。并行任务物理隔离,
   绝不共用工作树(否则抢 index / 文件 / `node_modules` / Vite HMR)。
4. **始终 rebase,不 merge**;`pull` 一律 `--ff-only`。保持 dev 线性历史、杜绝你制造的 merge 提交
   与意外合并。
5. **落地前必过 `preland` 强校验,落地走 `land`**(见 §4/§5);靠 git 原子 ref 更新无锁串行化,永不双头。
6. **冲突域控制**(见 §7):任务范围尽量收敛到明确文件集合;高冲突文件(migration / lockfile /
   全局类型 / app shell / 路由表)**不可多 session 并行修改**,需串行或先协调。

---

## 2. 拓扑总览

| 角色 | 位置 / 分支 | 谁写 | 写法 |
|------|------|------|------|
| 集成主干 | `origin/dev` | AutoForge / 同事 / 你的每个 session | 各自 rebase 后 push |
| 本地镜像 | 主仓 `~/projects/AutoForge`,签出 `dev` 且**干净** | 没人直接写 | 仅 `pull --ff-only` |
| 任务 A | worktree `../AutoForge-wt/feat-a`,分支 `feat/a` | Claude session #1 | 独占 |
| 任务 B | worktree `../AutoForge-wt/feat-b`,分支 `feat/b` | Claude session #2 | 独占 |
| 任务 … | 每任务一棵 | 每个 session 一棵 | 独占 |

---

## 3. 每个并行任务的生命周期

### 3.1 开树(从最新 origin/dev 起,一任务一棵)

```bash
bash scripts/branch/wt-new.sh <简短任务名>     # 打印 worktree 路径
```

它等价于:`git fetch origin dev && git worktree add ../AutoForge-wt/<slug> -b feat/<slug> origin/dev`。

> **不要**依赖 Claude Code `EnterWorktree` 的 `fresh` 基点——它从 `origin/<默认分支>` 建,
> 而本仓默认分支是 **main 不是 dev**。务必走 `wt-new.sh`(基于 `origin/dev`)。

### 3.2 在该树内启动一个 Claude session

```bash
cd <wt-new 打印的路径> && claude
```

或在已有 session 内 `EnterWorktree(path=<该路径>)`。该 session 整段只碰这棵树。

### 3.3 干活期间勤同步(把冲突摊小摊勤)

**不是"每天一次",而是在这三个时机立即 rebase**:

- **每次准备 commit 前**;
- **每次 land 前**(`land` 会自动再 rebase 一次兜底);
- **一发现 `origin/dev` 被 AutoForge/同事推进**(可 `git fetch` 后看 behind 数)。

```bash
git fetch origin && git rebase origin/dev
```

长跑分支设**时限**:最长半天到一天必须 rebase 一次,避免分叉积累成大冲突。

### 3.4 收尾(**仅在 land 成功且远端确认后**)

```bash
bash scripts/branch/wt-clean.sh     # 只删:工作树干净 且 origin/dev..HEAD 为 0(已落地)的 worktree
```

`wt-clean` 绝不删主仓、不删有未提交改动或有未落地提交的树,因此可放心跑。**不要**在 land 成功前手动
`worktree remove` / `branch -D`,以免误删未落地分支。

### 3.5 刷新本地镜像(随时,永远 ff-only)

```bash
git -C ~/projects/AutoForge switch dev && git -C ~/projects/AutoForge pull --ff-only origin dev
```

> 等价于 Settings 里「同步更新」按钮做的 ff-only 拉取。

---

## 4. 落地协议:preland 强校验 + rebase-then-push + 拒绝即重试

**这是多写入者并行的关键**——不需要任何外部锁,靠 git 原子 ref 更新天然串行化:
谁先 push 谁赢,后到者被拒 → 自动 rebase + 重跑校验 + 重试。

落地一律走 `land`(脚本全文见 §9):

```bash
bash scripts/branch/land.sh
```

`land` 的流程:
1. 结构快检(非 dev 分支、工作树干净);
2. `git fetch origin dev` 后 `git rebase origin/dev`(吸收最新);
3. **在已 rebase 的结果上跑 `preland` 强校验**(§5);未过则中止,**不 push**;
4. `git push origin HEAD:dev`;被拒(别人抢先)→ 自动 `fetch + rebase + 重跑 preland`,最多 5 次。

**两个 session 同时收工的真实时序:**

1. #1 和 #2 都基于 `origin/dev=A` 完成、rebase、push;
2. #1 push 成功 → `origin/dev=B`;
3. #2 push 被拒(非快进,它还基于 A)→ 自动 `fetch`(拿到 B)→ `rebase` 到 B → 重跑 preland → 再 push → `origin/dev=C`。

任意时刻 `origin/dev` 只有一个真实 head,每个写入者落地前都对齐过它 → **永不出现双头**。
AutoForge 的 L0 串行合并锁 + L1 合并前 merge dev 是同一套"先对齐再落地"哲学,你们彼此互为上游,自然兼容。

---

## 5. preland 落地前强校验

`land` 在 rebase 后自动调用;也可随时独立自检:`bash scripts/branch/preland.sh`(脚本全文见 §9)。
它只 `fetch`、**只读**(不 rebase、不 push),逐项检查:

| 检查 | 不过的后果 |
|------|------|
| 当前分支不是 `dev`、非 detached | 阻断 |
| 工作树干净、改动全部已提交 | 阻断(rebase 会失败/丢改动) |
| 无冲突标记 `<<<<<<<` / `>>>>>>>` | 阻断 |
| `git diff --check`(空白/标记错误) | 警告 |
| HEAD 已基于最新 `origin/dev`(`merge-base --is-ancestor`) | 阻断(让 land 去 rebase) |
| **高冲突文件预警**:列出本次相对 `origin/dev` 改动里命中 migration/lockfile/全局类型/app shell/路由/`index.css` 的文件 | 警告(提醒确认无并行修改,见 §7) |
| 可选测试/lint(设 `AUTOFORGE_PRELAND_CMD`,如 `"npm run lint && cargo check"`) | 阻断 |

> 建议在 shell profile 里设 `export AUTOFORGE_PRELAND_CMD="npm run lint && cargo check"`(或本项目对应的快速检查),
> 让 land 前自动跑;全量测试太慢可只放 lint + 类型检查,重测交给 AutoForge 流水线。

---

## 6. 让它无痛的开关与注意点

1. **开 rerere,冲突解一次终身复用**(多 session 反复 rebase 同段 dev 时尤其值):
   ```bash
   git config --global rerere.enabled true
   ```
   AutoForge 的 L1 合并已在用 rerere,开启后人机解法风格一致。

2. **依赖别每棵树重装**:
   - Rust:所有 worktree 共享一份 target——`export CARGO_TARGET_DIR=~/.cache/autoforge-target`
     (写进 shell profile),省磁盘省编译。
   - Node:新树 `node_modules` 可软链主仓(依赖未变时)或用 pnpm。
     注意 AutoForge 的 `core/dep_cache.rs` 只服务它自建的 worktree,**你的个人 worktree 依赖自己管**。

3. **分支命名留出处**:`feat/<主题>`、`fix/<主题>`;避免与 AutoForge 自动建的 CR 分支
   (用 CR id 命名)撞名。

4. **始终 rebase 不 merge;`pull` 一律 `--ff-only`**——从源头杜绝意外 merge 与双头。

5. **绝不在主仓里直接编辑/提交**;主仓只用来 `pull --ff-only` 看最新、跑 dev 预览。

6. **环境变量**:`AUTOFORGE_DEV_BRANCH`(默认 `dev`)、`AUTOFORGE_WT_DIR`(worktree 落点,默认
   `<主仓父目录>/<主仓名>-wt`)、`AUTOFORGE_PRELAND_CMD`(preland 测试命令)、
   `AUTOFORGE_GUARD_OFF=1`(临时关闭硬拦截,仅用于维护本机制本身,见 §8)。

---

## 7. 冲突域控制规则(降低"编辑冲突")

§1–§6 消除的是集成冲突;真正撞同一段代码的**编辑冲突**靠下面的约定 + preland 的自动预警:

1. **一次任务尽量收敛在明确的文件集合内**;大任务先拆成多个 **vertical slice**(每片独立可落地),
   而不是一个长跑分支横扫全仓。
2. **高冲突文件不可多 session 并行修改**——命中以下任一类时,**串行执行或先口头/看板占用**:
   - **数据库迁移** `src-tauri/migrations/*`(序号递增,天生易撞,且不可改已有文件);
   - **锁文件** `package-lock.json` / `pnpm-lock.yaml` / `yarn.lock` / `Cargo.lock`;
   - **全局类型 / 模块导出** `**/mod.rs`、公共类型定义;
   - **app shell / 注册入口** `src/App.tsx`、`src-tauri/src/lib.rs`、`src-tauri/src/state.rs`;
   - **IPC 封装层** `src/services/index.ts`、**设计系统** `src/index.css`、路由表。
3. **schema / migration 类任务串行**:同一时间只让一个 session 动 migration;它落地后其它 session 先
   `rebase origin/dev` 再继续,避免迁移序号冲突。
4. **长跑分支设时限**:半天到一天内必须 rebase;越久越容易和高冲突文件撞。
5. **preland 会自动预警**:落地前若改动触及上述高冲突文件,会列出来提示你确认无并行修改——
   看到预警就主动核对其它 session/同事,必要时改为串行。

> 说明:真正的"跨 session 硬锁"(把占用 claim 推到一条共享分支)成本高、收益有限,本规范**不强制**,
> 留作后续可选增强。当前以"约定 + preland 自动预警 + 高冲突文件串行"覆盖绝大多数编辑冲突。

---

## 8. 自动托管(hooks)——你几乎不用记命令

`.claude/settings.json` 注册了两个 harness 钩子(脚本全文见 §9):

- **SessionStart → `session_start.py`**:刷新 `origin/dev`,并按你所在位置注入指引:
  - 在**主仓 dev** → 提示"本次要改代码就先 `wt-new` + `EnterWorktree`,纯问答可留下但别写文件";
  - 在**隔离 worktree** → 提示 rebase 节奏、`land`、`preland`、高冲突文件注意。
- **PreToolUse(Bash / Write / Edit / NotebookEdit)→ `guard.py`**:**硬拦截(deny)**以下违规:
  - 在主仓 `dev` 只读镜像上编辑文件;
  - 在 `dev` 分支上 `git commit`;
  - 裸 `git pull`(非 `--ff-only` / `--rebase`);
  - `git merge dev`(应 rebase);
  - force push / push 到 `main|master` / 直接 push 到 `dev`(应走 `land`);
  - 在主仓把 `dev` 切走(破坏 `dev_is_live` 安全路径)。
  - 放行 `scripts/branch/{land,wt-new,wt-clean}`(内部自管);采用"同段 `git`+动词"匹配,避免误拦 echo/heredoc。

**逃生阀**:维护本机制本身(改 guard / 脚本 / 本文档,需在主仓编辑)时,以
`AUTOFORGE_GUARD_OFF=1 claude` 启动,临时放行。

> 钩子对**新启动**的 session 自动生效;**已开着**的 session 需打开一次 `/hooks` 或重启加载。

### 8.1 首次启用(bootstrap)——让主仓回到"干净镜像"

引入本机制时,主仓 dev 上会有未跟踪的 `BRANCHING.md`、`scripts/branch/*`、`.claude/settings.json`、
`CLAUDE.md` 改动等(不满足"干净镜像"假设)。一次性收敛步骤:

```bash
# 用逃生阀,在主仓把这套基建提交并落地到 origin/dev(仅此一次的 bootstrap)
AUTOFORGE_GUARD_OFF=1 claude     # 或手工:
#   git switch -c feat/branching-infra && git add BRANCHING.md scripts/branch .claude/settings.json CLAUDE.md \
#   && git commit -m "chore: 引入分支管理规范与自动托管" && bash scripts/branch/land.sh
```

落地后 `git -C ~/projects/AutoForge switch dev && git pull --ff-only`,主仓即回到干净只读镜像态。
此后一切开发都走 §3 的 worktree 流程。

---

## 9. 脚本附录(`scripts/branch/`)

> 真源是 `scripts/branch/` 下的实际文件;此处全文便于审阅与查阅。改脚本后请同步本节。

### 9.1 `wt-new.sh` — 建隔离 worktree

```bash
#!/usr/bin/env bash
# wt-new <任务名> —— 从最新 origin/<dev> 建一棵隔离 worktree + feature 分支,打印其路径。
set -euo pipefail
DEV="${AUTOFORGE_DEV_BRANCH:-dev}"
NAME="${1:-}"; [ -z "$NAME" ] && NAME="task-$(date +%m%d-%H%M%S)"
SLUG=$(printf '%s' "$NAME" | tr ' /' '--' | tr -cd 'A-Za-z0-9._-')
[ -z "$SLUG" ] && SLUG="task-$(date +%m%d-%H%M%S)"
COMMON=$(git rev-parse --path-format=absolute --git-common-dir); MAIN=$(dirname "$COMMON")
WTDIR="${AUTOFORGE_WT_DIR:-$(dirname "$MAIN")/$(basename "$MAIN")-wt}"; mkdir -p "$WTDIR"
DEST="$WTDIR/$SLUG"
[ -e "$DEST" ] && { echo "✗ 目标已存在: $DEST" >&2; exit 1; }
git -C "$MAIN" fetch origin "$DEV" >&2
git -C "$MAIN" worktree add "$DEST" -b "feat/$SLUG" "origin/$DEV" >&2
echo "✓ 已创建 worktree: $DEST(分支 feat/$SLUG,基点 origin/$DEV)" >&2
printf '%s\n' "$DEST"   # stdout 只输出纯路径,便于捕获
```

### 9.2 `preland.sh` — 落地前强校验

```bash
#!/usr/bin/env bash
# preland —— 落地前强校验。全过才允许 land。只 fetch、只读:不 rebase、不 push。
set -uo pipefail
DEV="${AUTOFORGE_DEV_BRANCH:-dev}"; fail=0
bad(){ printf '  ✗ %s\n' "$*"; fail=1; }; ok(){ printf '  ✓ %s\n' "$*"; }; note(){ printf '%s\n' "$*"; }
CUR=$(git branch --show-current)
{ [ -z "$CUR" ] && bad "detached HEAD"; } || { [ "$CUR" = "$DEV" ] && bad "当前在 $DEV" || ok "分支 $CUR"; }
[ -n "$(git status --porcelain)" ] && bad "工作树有未提交改动" || ok "工作树干净"
git grep -nI -e '^<<<<<<<' -e '^>>>>>>>' -- . >/dev/null 2>&1 && bad "检测到冲突标记" || ok "无冲突标记"
git diff --check HEAD >/dev/null 2>&1 || note "  ⚠️ git diff --check 报告空白/标记问题"
git fetch origin "$DEV" >/dev/null 2>&1 || true
git rev-parse -q --verify "origin/$DEV" >/dev/null && {
  git merge-base --is-ancestor "origin/$DEV" HEAD && ok "已基于最新 origin/$DEV" || bad "落后 origin/$DEV,请先 rebase"; }
# 高冲突文件预警
BASE=$(git merge-base "origin/$DEV" HEAD 2>/dev/null || echo "")
[ -n "$BASE" ] && { HOT=$(git diff --name-only "$BASE"..HEAD | grep -E \
  'migrations/|(package-lock\.json|pnpm-lock\.yaml|yarn\.lock|Cargo\.lock)$|services/index\.ts|App\.tsx|lib\.rs|state\.rs|mod\.rs$|index\.css' || true)
  [ -n "$HOT" ] && { note "  ⚠️ 触及高冲突文件,确认无并行修改:"; printf '%s\n' "$HOT" | sed 's/^/      - /'; }; }
# 可选测试/lint
[ -n "${AUTOFORGE_PRELAND_CMD:-}" ] && { note "  → $AUTOFORGE_PRELAND_CMD"; bash -c "$AUTOFORGE_PRELAND_CMD" && ok "测试/lint 通过" || bad "测试/lint 失败"; } \
  || note "  ℹ️ 未设 AUTOFORGE_PRELAND_CMD,跳过测试/lint"
[ "$fail" = 0 ] && { note "✅ preland 通过"; exit 0; } || { note "❌ preland 未通过"; exit 1; }
```

### 9.3 `land.sh` — 落地到 origin/dev

```bash
#!/usr/bin/env bash
# land —— preland 强校验 + rebase-then-push + 拒绝即重试。
set -euo pipefail
DEV="${AUTOFORGE_DEV_BRANCH:-dev}"; HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CUR=$(git branch --show-current)
[ -z "$CUR" ] && { echo "✗ detached HEAD"; exit 1; }
[ "$CUR" = "$DEV" ] && { echo "✗ 你在 $DEV 上,需从 feature 分支 land"; exit 1; }
[ -n "$(git status --porcelain)" ] && { echo "✗ 工作树有未提交改动,先 commit"; exit 1; }
git fetch origin "$DEV"
git rebase "origin/$DEV" || { echo "⚠️ 冲突,解决后 git rebase --continue 再重跑 land"; exit 1; }
bash "$HERE/preland.sh" || { echo "✗ preland 未过,已中止(未 push)"; exit 1; }
for i in 1 2 3 4 5; do
  git push origin "HEAD:$DEV" && { echo "✅ 已落地 origin/$DEV"; exit 0; }
  echo "↻ origin/$DEV 被推进,自动 rebase 重试 ($i/5)…"
  git fetch origin "$DEV"
  git rebase "origin/$DEV" || { echo "⚠️ 重试冲突,解决后 git rebase --continue 再 land"; exit 1; }
  bash "$HERE/preland.sh" || { echo "✗ rebase 后 preland 未过,已中止"; exit 1; }
done
echo "✗ 连续 5 次被抢先,稍后重试"; exit 1
```

### 9.4 `wt-clean.sh` — 清理已落地的 worktree

```bash
#!/usr/bin/env bash
# wt-clean —— 仅删:工作树干净 且 origin/<dev>..HEAD 为 0(已落地)的个人 worktree。绝不删主仓/未落地树。
set -euo pipefail
DEV="${AUTOFORGE_DEV_BRANCH:-dev}"
COMMON=$(git rev-parse --path-format=absolute --git-common-dir); MAIN=$(dirname "$COMMON")
git -C "$MAIN" fetch origin "$DEV" >/dev/null 2>&1 || true
git -C "$MAIN" worktree list --porcelain | awk '/^worktree /{print $2}' | while read -r WT; do
  [ "$WT" = "$MAIN" ] && continue
  BR=$(git -C "$WT" branch --show-current 2>/dev/null || echo "")
  DIRTY=$(git -C "$WT" status --porcelain 2>/dev/null || echo "x")
  AHEAD=$(git -C "$WT" rev-list --count "origin/$DEV..HEAD" 2>/dev/null || echo "?")
  if [ -z "$DIRTY" ] && [ "$AHEAD" = "0" ]; then
    echo "🧹 prune $WT ($BR)"; git -C "$MAIN" worktree remove "$WT"
    [ -n "$BR" ] && git -C "$MAIN" branch -D "$BR" 2>/dev/null || true
  else
    echo "✋ keep  $WT (分支=$BR, 未落地提交=$AHEAD, 有改动=$([ -n "$DIRTY" ] && echo 是 || echo 否))"
  fi
done
```

> `session_start.py`(SessionStart 指引)与 `guard.py`(PreToolUse 硬拦截)较长,见
> `scripts/branch/` 实际文件;其规则已在 §8 完整列出。

---

## 10. Do's and Don'ts

**✅ Do**

- 把 `origin/dev` 当唯一权威主干,落地前必先 rebase 对齐。
- 主仓 `dev` 保持签出且干净,只 `pull --ff-only`。
- 一任务一 worktree 一 feature 分支一 Claude session,物理隔离。
- 提交前 / 落地前 / 发现 dev 更新即 `git fetch && git rebase origin/dev`(别每天才一次)。
- 落地统一走 `land`(内置 `preland` 强校验 + 自动重试);设 `AUTOFORGE_PRELAND_CMD` 接 lint/类型检查。
- 大任务拆 vertical slice;高冲突文件(migration/lockfile/全局类型/app shell/路由)串行或先占用。
- 仅在 land 成功且远端确认后 `wt-clean`。开 `rerere.enabled`,共享 `CARGO_TARGET_DIR`。

**❌ Don't**

- ❌ 在本地 `dev` 上直接提交(制造双头的根源)。
- ❌ 把主仓切到非 dev 分支(触发 `dev_is_live=false` 的主工作树 in-place 合并,搅乱主仓)。
- ❌ 多个 session 共用同一工作树;❌ 多 session 并行改同一高冲突文件/同一 migration。
- ❌ 用裸 `git pull`(merge 模式)或 `git merge dev` 同步(产生 merge 提交、非线性历史)。
- ❌ 直接 `git push` 到 dev(走 `land`);❌ push main/master;❌ force push。
- ❌ 跳过 `preland` 直接推;❌ land 成功前 `worktree remove` / `branch -D`。
- ❌ 给 AutoForge 改 `project.branch_dev` 走单独集成分支——会因 `dev_is_live` 启发式
  导致它在主工作树本地合并,得改后端逻辑,得不偿失。

---

## 11. 一句话总结

`origin/dev` = 唯一权威主干;**主仓 dev = 干净只读镜像**;**一任务一 worktree 一 feature 分支一 session**
物理隔离;落地走 `land` = **rebase → preland 强校验 → push,被拒自动重试**;**冲突域控制**让高冲突文件串行。
无锁、原子、线性、永不双头,且把编辑冲突也压到最低——AutoForge 与同事随便推 dev,都能被各 session 勤 rebase 吸收。
