---
name: AutoForge
version: alpha
description: >-
  AutoForge 桌面端设计系统 —— 温暖的「熔炉/余烬」主题，深色为默认。
  以 ember 橙为唯一强调色，Archivo + Noto Sans SC + JetBrains Mono 三字族，
  圆角柔和、阴影克制，所有颜色与尺寸均以 CSS 变量为唯一真源（src/index.css）。
  支持 dark/light 双模式 × 多套 palette 主题切换。

colors:
  # 强调色（ember / 余烬）—— 全局唯一品牌色，亮色模式自动加深
  ember: "#e8772e"
  emberSoft: "#f29a55"
  emberDeep: "#c2571a"
  molten: "linear-gradient(135deg, #f4a93c 0%, #e8772e 55%, #d65f1c 100%)"
  emberTint: "rgba(232,119,46,.14)"
  emberTintStrong: "rgba(232,119,46,.22)"

  # 语义状态色（仅用于状态表达，不作装饰）
  green: "#4f9d6b"
  greenSoft: "#6fc090"
  blue: "#4f8ed1"
  violet: "#8b7ad8"
  red: "#db5a40"
  amber: "#e0a32e"

  # 表面层级（dark 默认，由深到浅：bg → surfaceActive）
  bg: "#16110d"
  bg1: "#1b150f"
  bg2: "#211a13"
  bg3: "#2a2017"
  surfaceHover: "#322619"
  surfaceActive: "#3a2c1c"
  codeBg: "#120d08"

  # 文本（由强到弱）
  text: "#f1e8d9"
  text2: "#b8a78f"
  text3: "#8a7964"
  textFaint: "#5f5343"

  # 描边
  border: "rgba(255,206,150,.10)"
  borderStrong: "rgba(255,206,150,.18)"

typography:
  display:
    fontFamily: '"Archivo", "Noto Sans SC", system-ui, sans-serif'
    usage: "页面标题 / 数字 KPI / 标志性强调，weight 700–800，letterSpacing 收紧"
  sans:
    fontFamily: '"Noto Sans SC", "Archivo", system-ui, -apple-system, sans-serif'
    usage: "正文与所有 UI 文案的默认字族"
  mono:
    fontFamily: '"JetBrains Mono", ui-monospace, "SFMono-Regular", monospace'
    usage: "代码、kicker/eyebrow、表单标签、chip、窗口标题等机械感文本"
  scale:
    micro: { fontSize: "10px" }      # 徽标、密集 chrome
    caption: { fontSize: "11px" }    # 元数据
    label: { fontSize: "12px" }      # 辅助/次要文本
    control: { fontSize: "13px" }    # 输入框、按钮、tab
    body: { fontSize: "14px", lineHeight: 1.5 }   # 默认正文
    title: { fontSize: "15px" }      # 行/卡片标题
    section: { fontSize: "16px" }    # 对话/区块标题
    heading: { fontSize: "18px" }    # 弹窗/报告标题
    pageTitle: { fontSize: "24px" }  # 设置/页面头
    metric: { fontSize: "28px" }     # KPI 数字
    display: { fontSize: "30px" }    # Dashboard hero
  leading:
    tight: 1.2
    snug: 1.35
    normal: 1.5
    relaxed: 1.6
    prose: 1.7

rounded:
  sm: "8px"
  md: "12px"     # --radius，默认圆角
  lg: "16px"
  xl: "22px"
  pill: "99px"   # chip / switch / 滚动条

spacing:
  xs: "4px"
  sm: "6px"
  md: "8px"
  lg: "12px"
  xl: "14px"
  "2xl": "16px"
  "3xl": "18px"
  railWidth: "68px"   # 导航 rail
  listWidth: "300px"  # 列表栏
  titlebar: "44px"    # 自定义标题栏高度

elevation:
  shadowSm: "0 1px 2px rgba(0,0,0,.18), 0 1px 1px rgba(0,0,0,.10)"
  shadow: "0 6px 20px rgba(0,0,0,.18)"
  shadowLg: "0 24px 60px rgba(0,0,0,.34)"
  window: "0 32px 80px rgba(0,0,0,.6), 0 0 0 1px rgba(255,255,255,.06)"

components:
  btn:
    backgroundColor: "{colors.bg3}"
    textColor: "{colors.text}"
    border: "1px solid {colors.borderStrong}"
    rounded: "{rounded.md}"   # 10px
    padding: "8px 14px"
    typography: "{typography.scale.control}"
    fontWeight: 600
  btnHover:
    backgroundColor: "{colors.surfaceHover}"
    border: "1px solid {colors.textFaint}"
  btnPrimary:
    backgroundColor: "{colors.molten}"
    textColor: "{colors.bg}"   # 深色文字压在熔岩渐变上
    border: none
    elevation: "0 3px 12px {colors.emberTintStrong}"
  btnGhost:
    backgroundColor: transparent
    border: "1px solid transparent"
  btnDanger:
    textColor: "{colors.red}"
    backgroundColor: "rgba(219,90,64,.10)"
  chip:
    typography: "{typography.scale.caption}"
    fontFamily: "{typography.mono.fontFamily}"
    rounded: "{rounded.pill}"
    padding: "3px 9px"
    border: "1px solid {colors.borderStrong}"
    variants: "ember | green | blue | violet | red | amber（语义着色，去边框 + tint 底）"
  card:
    backgroundColor: "{colors.bg2}"
    border: "1px solid {colors.border}"
    rounded: "{rounded.lg}"   # 14–16px
    padding: "13px 14px"
  panel:
    backgroundColor: "{colors.bg2}"
    border: "1px solid {colors.border}"
    rounded: "{rounded.lg}"   # 16px
    headerBorderBottom: "1px solid {colors.border}"
  stat:
    backgroundColor: "{colors.bg2}"
    rounded: "14px"
    valueTypography: "{typography.scale.metric}"
    valueFontFamily: "{typography.display.fontFamily}"
    valueFontWeight: 800
  field:
    backgroundColor: "{colors.bg3}"
    border: "1px solid {colors.borderStrong}"
    rounded: "9px"
    padding: "9px 11px"
    labelTypography: "{typography.scale.label}"
    labelFontFamily: "{typography.mono.fontFamily}"
  fieldFocus:
    border: "1px solid {colors.ember}"
    elevation: "0 0 0 3px {colors.emberTint}"
  switch:
    width: "42px"
    height: "24px"
    rounded: "{rounded.pill}"
    offColor: "{colors.borderStrong}"
    onColor: "{colors.ember}"
  seg:
    backgroundColor: "{colors.bg3}"
    rounded: "{rounded.md}"
    activeBackground: "{colors.bg}"
    activeElevation: "{elevation.shadowSm}"
  railItem:
    size: "46px"
    rounded: "13px"
    color: "{colors.text3}"
    activeBackground: "{colors.emberTintStrong}"
    activeColor: "{colors.emberSoft}"
  mentionPop:
    backgroundColor: "{colors.bg2}"
    rounded: "{rounded.md}"
    rowRounded: "9px"
    rowHoverBackground: "{colors.surfaceHover}"
    note: "项目/Agent 选择与下拉的统一模式，替代原生 <select>"
---

# AutoForge 设计系统（DESIGN.md）

> 本文件是 AutoForge 桌面端 UI 的**设计契约**。所有页面（Dashboard / Conversations /
> Projects / Audit / Settings）必须遵守此处的 token 与组件规范。
> **唯一真源是 `src/index.css` 的 CSS 变量** —— 本文件描述其意图与用法，
> 新增 UI 一律引用变量，禁止硬编码颜色与字号。

## Overview

AutoForge 是一个「Human-Lite-in-the-Loop」自主软件工厂的 Tauri 桌面应用。
设计语言围绕一个隐喻：**熔炉（Forge）与余烬（Ember）** —— AI 在后台「锻造」代码，
界面呈现温暖、专注、克制的工业感，而非冷冰冰的工具或花哨的消费品。

核心气质：

- **温暖深色优先**：默认深色，所有中性色带暖橙倾向（不是纯灰/纯黑），营造炉火旁的环境感。
- **单一强调色**：只有 ember 橙是品牌色。绿/蓝/紫/红/琥珀仅作**语义状态**，不作装饰点缀。
- **类原生桌面壳**：macOS 风格红绿灯按钮 + 自定义标题栏 + 圆角窗口（14px），整窗带强阴影。
- **机械感细节**：等宽字体用于 kicker、标签、窗口标题、chip，带大字距与大写，呼应「工厂/终端」主题。
- **安静而非空洞**：留白充足、动效微妙（仅 rise 入场、typing 等少量），尊重 `prefers-reduced-motion`。

整体布局是经典三栏桌面结构：`titlebar` → (`rail` 68px ｜ `list-col` 300px ｜ `content` 自适应)。

## Colors

**强调色 — ember（余烬）**：全局唯一品牌色。实色用 `--ember`，主按钮等用 `--molten`
熔岩渐变，背景着色用 `--ember-tint` / `--ember-tint-strong`。亮色模式下 ember 自动加深以保对比。

```
--ember #e8772e   --ember-soft #f29a55   --ember-deep #c2571a
--molten linear-gradient(135deg, #f4a93c, #e8772e, #d65f1c)
--ember-tint rgba(232,119,46,.14)   --ember-tint-strong rgba(232,119,46,.22)
```

**语义状态色**：只在表达状态/类别时使用（成功、进行中、错误、信息……），不得当装饰色铺设。

```
--green 成功/已合并   --amber 进行中/待处理   --red 失败/危险
--blue 信息   --violet 次级分类   （各自带 *-soft 与 14% tint 变体）
```

**表面层级（dark 默认）**：用层级而非边框堆叠来区分容器。由深到浅：

```
--bg #16110d → --bg-1 #1b150f → --bg-2 #211a13 → --bg-3 #2a2017
--surface-hover #322619   --surface-active #3a2c1c   --code-bg #120d08
```
- `--bg` 内容区底；`--bg-1` 列表栏；`--bg-2` 卡片/面板；`--bg-3` 输入框/分段控件/按钮底。

**文本**：四级灰阶，均带暖调。`--text` 主文 / `--text-2` 次要 / `--text-3` 辅助、标签 / `--text-faint` 最弱、占位。

**描边**：`--border`（极淡，默认分隔）与 `--border-strong`（输入框、可交互边界）。描边是带暖橙的半透明白，不是纯灰。

**主题机制**：`[data-theme="dark|light"]` × `[data-palette="carbon|moss|harbor|rose|indigo|volt|copper|glacier|plum|saffron|abyss|…"]`
每套 palette 重定义全部变量（含把 ember 替换为该主题的强调色）。**编写组件时只引用变量名，主题切换将自动生效**——
绝不在组件里写死十六进制色。

## Typography

三字族，各司其职：

| 字族 | 变量 | 用途 |
|------|------|------|
| Display | `--font-display`（Archivo） | 页面标题、KPI 数字、eyebrow、标志强调，weight 700–800 |
| Sans | `--font-sans`（Noto Sans SC） | **默认**：正文与一切 UI 文案，中英文混排 |
| Mono | `--font-mono`（JetBrains Mono） | 代码、kicker、表单 label、窗口标题、chip —— 带大字距 + 大写 |

字号阶梯（变量 → px）：`micro 10 · caption 11 · label 12 · control 13 · body 14 · title 15 · section 16 · heading 18 · page-title 24 · metric 28 · display 30`。
行高用 `--leading-*`（tight 1.2 → prose 1.7），正文默认 `--leading-normal 1.5`，气泡/长文用 relaxed/prose。

惯用搭配：
- **页面头/Hero**：`--font-display` + `--text-display/page-title` + 收紧字距（`-.01em`）。
- **KPI 数字**：`--font-display` weight 800 + `--text-metric` + `line-height: 1`。
- **kicker / 标签**：`--font-mono` + `--text-caption/label` + `letter-spacing .14–.18em` + `text-transform: uppercase`。
- **正文**：`--font-sans` + `--text-body`。

## Layout

应用壳为固定三栏 + 顶栏：

```
┌─ os-titlebar (44px, 毛玻璃, 红绿灯 + 居中 mono 标题) ─────────────┐
│ rail │ list-col │            content                              │
│ 68px │  300px   │           flex: 1                               │
└──────┴──────────┴────────────────────────────────────────────────┘
```

- **rail**（`--rail-w 68px`）：图标导航，48px 触达项，激活态用 `--ember-tint-strong` 底 + `--ember-soft` 字 + 左侧 ember 指示条。
- **list-col**（`--list-w 300px`）：会话/项目/需求列表，底为 `--bg-1`，含搜索框与 `list-title`（display 18px）。
- **content**：主工作区，底为 `--bg`，承载页面内容。

间距尺度以 2/4 为基：常用 `gap` 6/8/12/14px，卡片 `padding` 13–18px，面板头 15×18px。
栅格：KPI 用 `repeat(4, 1fr)` + 12px gap；表单字段用 `1fr 1fr` + 12px gap（`.field.full` 跨整行）。

## Elevation & Depth

阴影克制、暖调、用于"浮起"而非装饰。四级：

```
--shadow-sm  细微浮起（分段控件 on 态、小卡）
--shadow     下拉/弹层/悬浮卡
--shadow-lg  模态/大弹层
窗口         0 32px 80px rgba(0,0,0,.6) + 1px 内描边白
```

深度优先用**表面层级**（bg → bg-3）表达，其次才用阴影。focus 态用 ember 光环
（`box-shadow: 0 0 0 3px var(--ember-tint)`）而非粗边框。毛玻璃仅用于 titlebar
（`backdrop-filter: blur(20px) saturate(1.4)`）。

## Shapes

圆角统一柔和：

```
--radius-sm 8   --radius 12（默认）   --radius-lg 16   --radius-xl 22   pill 99
```

- 窗口与 `#root`：14px。卡片/面板：14–16px。按钮：10px。输入框：9px。rail 项：13px。
- 全圆（pill）：chip、switch、状态 dot、滚动条 thumb。
- 头像 `.av`：13px 圆角方形（非圆形），右下角状态点带 2.5px 描边。

## Components

> 组件类已在 `src/index.css` 定义；新页面**复用类名**，不要另起炉灶写平行样式。

- **Button `.btn`**：bg-3 底 + border-strong 边 + 10px 圆角，control 字号 600。
  - `.btn-primary` 熔岩渐变 + 深色字 + ember 阴影（页面主操作，每屏≤1个主按钮）。
  - `.btn-ghost` 透明；`.btn-danger` 红色危险操作；`.btn-sm` 紧凑；`.icon-btn` 32px 方形图标按钮。
- **Chip `.chip`**：mono 字 + pill 形 + caption 字号，语义变体 `.ember/.green/.blue/.violet/.red/.amber`（去边框 + tint 底）。
- **Dot `.dot`**：状态小圆点，`.green/.amber` 带光环，`.gray` 用于离线/禁用。
- **Card / Panel**：`.panel` = bg-2 + border + 16px 圆角 + `.panel-head`（带底分隔）。统计卡 `.stat` 用 icon|main|delta 网格。
- **Segmented `.seg`**：分段切换，bg-3 容器，激活段 `.on` 提升到 bg 底 + shadow-sm。**用它替代 tab 切换**。
- **Field `.field`**：纵向 label + 控件，label 为 mono 大写小字；输入 bg-3 + border-strong + 9px 圆角，focus 转 ember 边 + 光环。
- **Switch `.switch`**：42×24 pill 开关，开态 ember 底。
- **下拉 / 提及 `.proj-select` + `.mention-pop` + `.mention-row`**：**唯一允许的下拉模式**，统一项目选择、@提及、菜单。
- **Avatar `.av`** / **Window shell `.os-window/.os-titlebar/.traffic`** / **导航 `.rail/.rail-item`** / **空态 `.empty`**：见 `src/index.css`。
- **消息块**（会议室）：`md / code / file / image / artifact / quote_ref / file_written`，渲染于 `src/components/Block.tsx`，气泡用 `--bubble-me`（熔岩）/ `--bubble-them`。

动效：入场 `.rise`（translateY 9px，.34s）、`typing` 三点、carousel 滚入。全部尊重
`prefers-reduced-motion` 与 `[data-motion="off"]`。交互过渡统一 .08–.18s。

## Do's and Don'ts

**✅ Do**

- 一切颜色、字号、圆角、阴影、间距都用 `src/index.css` 的 CSS 变量。
- 用表面层级（bg→bg-3）表达深度，强调只用 ember。
- 图标统一走 `<Icon name="..." />`（`src/components/Icon.tsx`）。
- 下拉/选择统一用 `proj-select + mention-pop + mention-row` 模式（参考 `Audit.tsx`）。
- kicker/标签/窗口标题用 mono + 大字距 + 大写，呼应工厂主题。
- 每屏至多一个 `.btn-primary` 主操作；语义色仅表达状态。
- 新增 block 类型时三处同步：`mock.ts` 类型、`Block.tsx` 渲染、Rust 侧 JSON。

**❌ Don't**

- ❌ 硬编码十六进制颜色或 px 字号（会破坏主题切换与一致性）。
- ❌ 使用原生 `<select>` 控件。
- ❌ 引入第二个品牌强调色，或把语义色（绿/蓝/紫）当装饰铺设。
- ❌ 引入 MUI/Antd 等第三方 UI 框架（仅 React + 自有 CSS）。
- ❌ 用纯灰/纯黑中性色（本系统中性色一律带暖橙倾向）。
- ❌ 滥用阴影与动效；深度优先用层级，动效保持微妙且可关闭。
- ❌ 在页面组件里写死与 `src/index.css` 平行的新样式体系。
