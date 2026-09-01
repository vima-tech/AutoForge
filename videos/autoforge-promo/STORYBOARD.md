---
format: 1920x1080
duration: 75s
mode: autonomous
message: "AutoForge 把普通程序员最弱的三件事工程化兜底——一人，就是一条流水线"
arc: "PAS：Hook → Pain → Product → Pipeline → 三重兜底 → Payoff → Brand"
audience: "普通程序员 / 小团队技术负责人（非 AI 极客）"
music: "dark industrial minimal tech underscore, warm ember pulse, confident, restrained"
---

## Video direction

- **palette system**（源自 frame.md，broadside × Ember 品牌混排）：canvas = 深暖墨 `#16110d`；panel = `#211a13` / `#2a2017`；ink = 暖白 `#f1e8d9`；secondary text = `#b8a78f`；唯一强调色 = ember 橙 `#e8772e`（关键动词、下划扫线、激活态）。语义色仅表状态：绿 `#4f9d6b` = 通过/打勾，红 `#db5a40` = 拦截/危险——不作装饰。display = Archivo Black（仅 400 档，其 400 即超粗字面，display 槽用 400 勿合成加粗）+ Noto Sans SC（CJK 700/800）；mono = JetBrains Mono（kicker、标签、代号、代码 chip，全大写 +0.14em）。
- **motion grammar**：一律 power3 长尾落定，不要 bounce；VO pacing 是唯一的揭示时钟——任何元素不得在其被念到之前出现，揭示铺进每帧后 50%；hold 期间唯一允许的活性是**低幅 subtle jitter**；机器工作态脉冲（spinner、脉冲点）只在 Frame 4 合法，且必须有限、在状态解决的那一帧准点熄灭。帧内接缝一律 velocity-matched cut。
- **rhythm / held frames**：1 快（钩子撞击）→ 2 稳（横移扫站）→ 3 克制（品牌 prelude）→ 4 密集（机器剧场，全片最忙）→ 5/6/7 中快节奏的"三重兜底"跑段（push-slide LEFT 链）→ **8 全片急停**（绝对静帧，唯一 breather）→ 9 收拢。Frame 8 的静是刻意设计，不是没动画。
- **negative list**：禁紫蓝"AI 感"渐变、禁浮焦光斑、禁圆环呼吸、禁后半程慢推/漂移镜头、禁 `back.out`/`elastic` 类弹跳、禁真实浏览器 chrome/滚动条、禁首页 25% 倾泻（slideshow）、禁多元素各自漂浮（screensaver）。真实光标全片禁用（Frame 4 用 already-working 触发，无光标）。
- **caption keep-out**：底部 ~17% 为字幕胶囊保留区，所有内容规划进顶部 ~83%。

## Frame 1 — 时间黑洞

- scene: 黑底中央大字节拍：「写代码」一词原地被「找代码」撞换
- voiceover: "你不是写不动代码。你只是把一半的时间，耗在了找代码上。"
- duration: 4.963s
- transition_in: cut
- status: animated
- src: compositions/frames/01-time-sink.html
- type: hook
- persuasion: Pain validation — 用观众自己的日常挫败开场，先证明"我懂你"
- beat: tension → recognition
- blueprint: kinetic-type-beats (Adapt — sub-shape A 固定行 token 撞换)
- focal: none — 纯排版帧
- roles: headline = hero · kicker = supporting
- sfx: impact-soft（token 撞换瞬间）
- asset_candidates:

Adapt：保留"固定行 + 原地硬切撞换"签名动作；改为两态句 + ember 下划扫线收束，无背景图案。
Scene 1 (0.0–1.2s): 纯色 canvas；左上 mono kicker `// 普通程序员的一天` 淡入；主标题「你不是写不动代码。」以 per-word staggered reveal（`dynamic-content-sequencing`）居中落定——Centered，主视觉 ~55% 画幅。
Scene 2 (1.2–3.2s): VO 走到"你只是把一半的时间"，整行硬切为「你只是把一半的时间，耗在了——」，尾部挂闪烁 caret；槽位里 ghost 态「写代码」（描边空心、40% 淡）闪现待命。
Scene 3 (3.2–4.96s): VO 念出"找代码"的一瞬，槽位 token 硬切撞换 写代码→找代码（`discrete-text-sequence`），ember 下划扫线从左到右画在「找代码」下（`css-marker-patterns`）；随后完全静止到帧尾。

narrativeRole: 冷开场钩子。不介绍产品、不摆数据，直接戳普通程序员最真实的日常——时间没花在写代码上。
keyMessage: 你的时间黑洞不是"写"，是"找"。

## Frame 2 — 三块短板

- scene: 横向站带镜头依次扫过三个站点：「定位·靠 grep」「纪律·靠自觉」「安全·靠运气」，终点收拢成死结「块块致命」
- voiceover: "定位，靠 grep。纪律，靠自觉。安全，靠运气。——三块短板，块块致命。"
- duration: 8.882s
- transition_in: zoom-through
- status: animated
- src: compositions/frames/02-three-gaps.html
- type: pain_point
- persuasion: Pain agitation + Rule of three — 把模糊的焦虑命名成三个具体短板，逐站加压
- beat: anxiety
- blueprint: spatial-pan-stations (Adapt — Problem 变体)
- focal: none — 示意图帧
- roles: 三站点 label+icon = hero · 引导线 = supporting · 死结 callout = payoff
- sfx: riser（收尾死结）
- asset_candidates:

Adapt：保留"单虚拟相机逐站对角横移 + 终点乱线死结"签名动作与 ember 手绘引导线（`svg-path-draw`）；站点改为 mono 线图标 + 短板标签卡。
Scene 1 (0.0–1.8s): 超宽画布；相机开局落在站点 1：放大镜线图标 +「定位 · 靠 grep」随到站自然揭示；ember 引导线从站点 1 向画面右下画出（`svg-path-draw`）。
Scene 2 (1.8–3.7s): 相机 ease-in-out 对角横移（`viewport-change` PAN + `coordinate-target-zoom` 无缩放）到站点 2：盾牌线图标 +「纪律 · 靠自觉」；引导线继续向前画。
Scene 3 (3.7–5.7s): 横移到站点 3：骰子线图标 +「安全 · 靠运气」；引导线继续。
Scene 4 (5.7–8.88s): 最后一次横移收尾：引导线在中央螺旋成密集乱线死结（`svg-path-draw` 潦草 knot）；死结下方 callout「三块短板，块块致命」克制 spring-pop（`spring-pop-entrance`）；相机锁死，静持到帧尾。

narrativeRole: 痛点激化。把 Frame 1 的模糊挫败拆成三个可指认的工程短板：代码定位、工程纪律、安全边界——为后面的"三重兜底"逐一埋钩子。
keyMessage: 定位靠 grep、纪律靠自觉、安全靠运气——这是系统性问题，不是你不努力。

## Frame 3 — AutoForge 登场

- scene: 三拍标题链：ember 核心点阵点燃 → 「AutoForge」字标拼合 → 标语卡「自主软件工厂 / 这三件事，全部焊死在架构里」
- voiceover: "AutoForge——自主软件工厂。这三件事，全部焊死在架构里。"
- duration: 5.512s
- transition_in: blur-crossfade
- status: animated
- src: compositions/frames/03-intro.html
- type: product_intro
- persuasion: Authority by assertion — 不解释原理，先立态度："焊死"是产品的承诺方式
- beat: intrigue → relief
- blueprint: titlecard-reveal (Reproduce — Product_Intro prelude 卡链)
- focal: none — 品牌排版帧
- roles: ember 核心点阵/字标 = hero · 标语卡 = payoff
- sfx: soft-ignite（ glow 点燃）
- asset_candidates:

Reproduce prelude 卡链：三卡各一个克制动作，blur-snap 手递手（`depth-of-field-blur`），末卡静持。
Scene 1 (0.0–1.5s): 净场深色；中心一点 ember glow 点燃（`ambient-glow-bloom` 单次），六枚小型方点从散开位置收拢成抽象核心阵列（`spring-pop-entrance` 克制形）+ 一次性模糊光环脉冲；它不是品牌 logo，不描摹任何官方标记。
Scene 2 (1.5–3.4s): blur-snap 交接：字标「AutoForge」居中（display 800，小写），mono 版本签 `alpha` 由灰转亮 append（`discrete-text-sequence`）；下方 mono kicker `HUMAN-LITE-IN-THE-LOOP · 自主软件工厂`。
Scene 3 (3.4–5.51s): blur-snap 到标语卡：「这三件事，全部焊死在架构里。」——「焊死」二字 ember 橙；静持到帧尾，无第二动作。

narrativeRole: 产品登场。痛点顶点处给出名字和一句话定位，"焊死在架构里"直接回应上一帧的三块短板——不是靠自觉，是靠架构。
keyMessage: AutoForge = 自主软件工厂；短板不是靠人补，是焊死在架构里。

## Frame 4 — 一人一条流水线（hero）

- scene: 触发后机器开工剧场：流水线卡片逐行点亮打勾「需求分析 ✓ → 代码实现(worktree) ✓ → 测试门 ✓ → 安全门 ✓ → squash 合并 ✓」，侧栏并发槽 5/5 轮转
- voiceover: "需求进来——分析、实现、测试、合并，全自动跑完。五个变更，并行推进。你，只管审核。"
- duration: 9.117s
- transition_in: zoom-through
- status: animated
- src: compositions/frames/04-pipeline.html
- type: feature_showcase
- persuasion: Show-don't-tell proof — 让观众看着机器把整条流水线跑完，状态翻转本身就是证据
- beat: awe → trust
- blueprint: agent-progress-theater (Adapt — already-working 触发 + checklist 剧场)
- focal: none — 重构 UI 示意帧
- roles: 流水线回执卡 = hero · 并发槽侧栏 = supporting · issue 角标 = setup
- sfx: tick × 5（每行打勾）、resolve-chime（merged）
- asset_candidates:

Adapt：保留"工作态剧场 + 回执卡逐行打勾状态突变"签名动作；改为 already-working 触发（无光标），深色锻造台 UI 替代浅底卡；右侧加 5 槽并发栏。机器脉冲全部有限、在解决帧熄灭。
Scene 1 (0.0–1.3s): 深色 canvas + 极淡点阵纹理；issue 卡「需求 · 批量导入 CSV」滑入并收缩停靠左上成角标；ember 弧形 spinner 在其旁点燃并有限旋转；mono 状态 pill `queued → analyzing` 翻转（`discrete-text-sequence`）。
Scene 2 (1.3–3.6s): 工作剧场：loader lockup 逐字打出「分析中…」（`discrete-text-sequence` + caret），状态对句「拆解需求…」「评估影响面…」在 spinner 下稳拍互换；右侧栏 5 枚槽位 chip `CR-1…CR-5` 依次脉冲进入 running（`sine-wave-loop` 有限脉冲）。
Scene 3 (3.6–7.8s): 回执卡居中展开，五行滑入：需求分析 / 代码实现 · worktree 隔离 / 测试门 / 安全门 / squash 合并；随 VO「分析、实现、测试、合并」逐行触发状态突变：编号描边徽标翻转为实心绿圆 + 白色对勾（`scale-swap-transition` + 克制回弹），已完成行标签划线变淡。
Scene 4 (7.8–9.12s): 收束：状态 pill 翻 `merged ✓`；侧栏 5/5 槽位全忙；VO 落"你，只管审核"时该行以 ember 在卡下定格；静持，spinner 与脉冲全部熄灭。

narrativeRole: 核心证明帧。整条流水线一镜跑完：需求分析→worktree 实现→测试门→安全门→合并，五行依次打勾；侧栏 5 个并发槽说明"一人同时推多条"。"你只管审核"埋 payoff。
keyMessage: 需求到合并全自动，一人看管一条流水线。

## Frame 5 — 兜底①：找代码

- scene: 情报卡栅格自组装：「符号定位 file:line」「调用者列表」「影响面分析」三张卡依次弹入，底部滑入「快模型 / 强模型·按风险分级」chip
- voiceover: "找代码？开工之前，符号已经定位到行——调用者、影响面，直接写进 prompt。小活快模型，大活强模型。"
- duration: 11.781s
- transition_in: push-slide LEFT
- status: animated
- src: compositions/frames/05-code-intel.html
- type: feature_showcase
- persuasion: Feature-to-benefit translation — 把 MCP 代码情报翻译成"不用再全仓 grep"
- beat: clarity
- blueprint: grid-card-assemble (Reproduce — Key_Feature grid 变体)
- focal: none — 卡片图形帧
- roles: 三情报卡 = hero · 分级 chip = payoff · headline = setup
- sfx: pop × 3（卡片入位）、whoosh（glow 扫过）
- asset_candidates:

Reproduce：2×2 砖块栅格逐卡短距装入 + 收尾一次性行光；每卡严格等它的 VO 提示词。
Scene 1 (0.0–1.8s): 深色 canvas；左上 mono kicker `MCP · CODE-INTEL 预查`；标题「开工之前，已经查完了。」逐行填出（`discrete-text-sequence`）。
Scene 2 (1.8–7.7s): VO 念"定位到行"→ 卡1 弹入（符号定位，mono code chip `tasks/merge.rs:696`）；念"调用者"→ 卡2 弹入（调用者列表两行 mini 行）；念"影响面"→ 卡3 弹入（影响面 mini 树）；短距直入栅格槽位（`center-outward-expansion` 短路径形，克制 spring，≤500ms stagger）。
Scene 3 (7.7–11.78s): VO「小活快模型，大活强模型」→ 底部滑入分级 pill：左半「低风险 → 快模型 · 省」右半「高风险 → 强模型 · 稳」，ember 分隔缝；一道 ember 行光从卡阵后单次扫过（`ambient-glow-bloom`）；落定静持。

narrativeRole: 兑现"兜底①"。回应 Frame 2 的「定位靠 grep」：执行前代码情报预查把符号定位到 file:line、调用者、影响面注入 prompt；分级选模型让成本和质量自动平衡。
keyMessage: 开工前，定位、调用者、影响面已经躺在 prompt 里；模型按风险自动分级。

## Frame 6 — 兜底②：纪律

- scene: 锚句「纪律，不靠自觉」钉在左侧不动，右侧护栏徽章轮播：「worktree 隔离」→「GitProxy 硬拦截」→「合并三重门」，末拍收拢「不遵守，就走不通」
- voiceover: "纪律，不靠自觉。worktree 隔离。危险命令，硬拦截。合并，三重门。——不遵守，就走不通。"
- duration: 11.572s
- transition_in: push-slide LEFT
- status: animated
- src: compositions/frames/06-guardrails.html
- type: feature_showcase
- persuasion: Risk reversal — 把"人会犯错"的默认假设反转成"架构不给你犯错的路径"
- beat: confidence
- blueprint: fixed-anchor-cycle (Reproduce — sub-shape A 邻区轮播)
- focal: none — 锚句+徽章图形帧
- roles: 锚句 = anchor（钉死不动）· 护栏 chip = cycling region · 收拢句 = payoff
- sfx: chip-slap × 3（徽章硬切）
- asset_candidates:

Reproduce：锚句全程零位移；邻区 chip 以硬切标签替换轮播，chip 宽度随文案重排（远离锚句方向生长）。
Scene 1 (0.0–2.4s): 锚句「纪律，不靠自觉。」一次性进入（`spring-pop-entrance` 克制），钉在画面左黄金位——此后零移动、零呼吸。
Scene 2 (2.4–8.9s): VO 每念一道护栏，右下 chip 硬切替换（`discrete-text-sequence`，约 2.1s/枚）：「worktree 隔离 · 主仓只读」→「GitProxy · 危险命令硬拦截」（念"硬拦截"时 chip 左缘亮一道红缝）→「合并三重门 · 测试 / 安全 / 落地再校验」；激活 chip 下一道 ember 短刻度线。
Scene 3 (8.9–11.57s): 强调拍：轮播骤停，收拢句「——不遵守，就走不通。」以 ember 落到与锚句同一基线；静持到帧尾。

narrativeRole: 兑现"兜底②"。回应 Frame 2 的「纪律靠自觉」：主仓只读 + worktree 隔离、GitProxy 危险命令硬拦截、合并前测试/安全/落地再校验三重门——高级工程纪律变成无法绕过的默认值。
keyMessage: 纪律不靠自觉——不遵守，就走不通。

## Frame 7 — 兜底③：安全

- scene: 重锤节拍蒙太奇：「注入·整条丢弃」「密钥·信封加密」「越界写入·直接拒绝」「失控进程·当场真杀」四条短句逐拍砸落清空
- voiceover: "安全，不靠运气。注入，整条丢弃。密钥，信封加密。越界写入，直接拒绝。失控进程，当场真杀。"
- duration: 14.054s
- transition_in: push-slide LEFT
- status: animated
- src: compositions/frames/07-security.html
- type: benefit_highlight
- persuasion: Risk reversal (value stacking) — 四个具体处置动作堆出"兜底到底"的安全感
- beat: confidence + control
- blueprint: kinetic-type-beats (Adapt — staccato 重锤拍)
- focal: none — 纯排版帧
- roles: 四组处置句 = hero（逐拍独占画面）
- sfx: percussive-hit × 4（每拍一记）
- asset_candidates:

Adapt：保留"闪现-清空"节拍签名；按真实配音节奏改为 ~2.2s/拍的重锤落点（每拍砸落、定格半拍、再硬切），处置词 ember、mono kicker 标子系统代号。
Scene 1 (0.0–1.8s): 引拍「安全，不靠运气。」居中落定，硬切清空。
Scene 2 (1.8–11.9s): 四拍连砸，每拍独占全屏中央（springy scale-in → 定格半拍 → 硬切清空，`kinetic-beat-slam`）：拍1 kicker `INJECTION` +「注入 → 整条丢弃」；拍2 `SECRETS` +「密钥 → 信封加密」；拍3 `PATH-GUARD` +「越界写入 → 直接拒绝」；拍4 `PROC-GUARD` +「失控进程 → 当场真杀」；箭头后处置词一律 ember。
Scene 3 (11.9–14.05s): 末拍「失控进程 → 当场真杀」不再清空，落定静持到帧尾。

narrativeRole: 兑现"兜底③"。回应 Frame 2 的「安全靠运气」：输入注入整条丢弃、密钥 AES-256-GCM 信封加密、工作区越界写入拒绝、失控子进程真杀——四个动词全是处置动作，零形容词。
keyMessage: 安全不靠运气，每一类事故都有对应的硬处置。

## Frame 8 — 你只做两个决策

- scene: 安静的标题卡：「你只做两个决策：/ 这个需求，要不要做。/ 这段代码，收不收。」一次克制的滑入后静定
- voiceover: "你只做两个决策：这个需求，要不要做；这段代码，收不收。"
- duration: 5.59s
- transition_in: zoom-through
- status: animated
- src: compositions/frames/08-two-decisions.html
- type: benefit_highlight
- persuasion: Future pacing — 把观众放进"审核者"的未来身份里，安静本身就是说服力
- beat: relief + control
- blueprint: titlecard-reveal (Reproduce — Benefits 变体)
- focal: none — 纯排版帧
- roles: 两行决策 = hero
- sfx: none（静默是本帧的设计）
- asset_candidates:

Reproduce：全片唯一 breather。一次滑入交叠是唯一的动作，之后是完全的静。
Scene 1 (0.0–1.4s): 净场；第一行「你只做两个决策：」淡入居中，轻微 95%→100% 缩放落定（`scale-swap-transition` 克制形）。
Scene 2 (1.4–2.9s): 唯一动作——滑入交叠（`discrete-text-sequence` translate-up + crossfade）：上行淡出，两行决策居中落定：「这个需求，要不要做。」/「这段代码，收不收。」（第二行 ember）。
Scene 3 (2.9–5.59s): 完全静持；至多 ember 行带一丝 subtle jitter（`sine-wave-loop` 低幅）。无镜头、无第二发展阶段。

narrativeRole: 价值落点（payoff）。前三帧的机械感在这里突然安静：机器跑完一切之后，人的位置被压缩到两个审核决策点——需求审核、代码审核。低动效是有意的呼吸口。
keyMessage: 人的价值，压缩到两个决策点。

## Frame 9 — 品牌收尾

- scene: 元素向中心收拢，AutoForge 官方图标显影锁定，字标「AutoForge」落定，下挂一行「一个人，就是一条流水线。」
- voiceover: "AutoForge——一个人，就是一条流水线。"
- duration: 3.344s
- transition_in: crossfade
- status: animated
- src: compositions/frames/09-brand.html
- type: cta
- persuasion: Value stacking resolve — 全片所有证明收拢成一句话身份宣言
- beat: triumph
- blueprint: logo-assemble-lockup (Adapt — parts-arrive 拼合·压缩版)
- focal: .media/images/logo_001.png — AutoForge 官方应用图标
- roles: 熔炉标记+字标 = hero · slogan = payoff
- sfx: resolve-shimmer（锁定瞬间）
- asset_candidates:

Adapt：保留"部件到场拼合 + 字标逐字落下"签名动作，但品牌标记必须使用官方图标资产，禁止手绘重构。按真实时长压缩——图标显影与字标级联并行进行，slogan 快速抹入即静持。
Scene 1 (0.0–0.6s): 净场深色；四条 ember 细线由画面边缘向中心汇拢，为官方图标预留落点（`svg-path-draw`，仅作引导线，不描摹 logo）。
Scene 2 (0.6–2.1s): 官方图标 `.media/images/logo_001.png` 以 clip-path 从中心向外显影并 96%→100% 落定；字标「AutoForge」字母左→右级联落下（`waterfall-entry`，无弹跳），与图标并行收束。
Scene 3 (2.1–3.34s): mono 小字 slogan「一个人，就是一条流水线。」左→右抹入（clip-path reveal）；静持到末帧。

narrativeRole: 品牌收尾。元素清场、标记拼合、字标落定，slogan 完成全片承诺的回扣：短板被兜底之后，一个人就是一条流水线。
keyMessage: AutoForge——一个人，就是一条流水线。
