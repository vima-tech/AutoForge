# AutoForge 栈品类完善实施计划

## 📊 现状评估

### 当前支持的品类（生产就绪）

#### 1. 前端品类
- **React / Vue / Angular / Svelte** ✅
  - 框架检测：`detect_node()` → `frontend_framework()`
  - 栈 ID：`node-frontend`
  - 命令：`npm/pnpm/yarn run dev | build | test`
  - 预览：Web iframe 嵌入
  - 包管理器自适应：npm/pnpm/yarn/bun

- **Next.js / Nuxt / SvelteKit**（SSR 框架）✅
  - 同 node-frontend，框架自动识别
  - dev_command 支持 {port} 占位

- **纯静态网站** ✅
  - 框架检测：`detect_static()` → index.html 存在 + 无 package.json
  - 栈 ID：`static`
  - 命令：`python3 -m http.server {port}`

#### 2. 后端品类

- **Spring Boot (Java/Maven/Gradle)** ✅
  - 框架检测：pom.xml/build.gradle + `spring-boot` 关键字
  - 栈 ID：`java-maven` / `java-gradle`
  - 命令：`mvn spring-boot:run` / `gradle bootRun` 支持 {port}
  - 安全扫描：OWASP Dependency-Check

- **Go (Gin / Echo / Fiber)** ✅
  - 框架检测：go.mod + 依赖分析
  - 栈 ID：`go`
  - 命令：`go run .`（PORT env）
  - 安全扫描：`govulncheck`

- **Python (FastAPI / Django / Flask)** ✅
  - 框架检测：requirements.txt/pyproject.toml + 关键字
  - 栈 ID：`python-pip` / `python-poetry`
  - 命令：`poetry run` / 裸命令，支持 {port}
  - 安全扫描：`pip-audit`

- **Node.js 后端 (Express / NestJS / Fastify)** ✅
  - 框架检测：package.json deps 无前端框架
  - 栈 ID：`node-backend`
  - 命令：`npm run dev/start`

- **Rust（独立或嵌入 Tauri）** ✅
  - 框架检测：Cargo.toml（非 src-tauri）
  - 栈 ID：`rust`
  - 命令：`cargo run/build`
  - 安全扫描：`cargo audit --no-fetch`

#### 3. 桌面品类

- **Tauri 2.x** ✅
  - 框架检测：src-tauri/tauri.conf.json + src-tauri/Cargo.toml
  - 栈 ID：`tauri`
  - 角色：`Frontend + Desktop` 混合
  - 命令：`npm run dev/tauri:dev`
  - 依赖软链：node_modules + src-tauri/target（避免冷编译）

---

## 🎯 新品类实施方案

### Phase 1: 微信小程序支持（优先级：高）

#### 1.1 技术方案

**检测机制** （在 `core/stack.rs` 新增 `detect_wechat_mini_app()`）

```rust
fn detect_wechat_mini_app(dir: &Path) -> Option<DetectedStack> {
    // 检测标志：project.config.json + app.json（小程序配置文件）
    let has_config = exists(dir, "project.config.json");
    let has_app = exists(dir, "app.json");
    if !has_config && !has_app {
        return None;
    }
    
    // 读 project.config.json 获取框架 & 语言
    let config = read_head(dir, "project.config.json", 5000)?;
    
    // 判定框架（互斥）：
    // - 官方原生：pages/ + .wxml + .wxss
    // - Taro：taro.config.js + node_modules/@tarojs
    // - uni-app：uni_modules + pages.json  
    // - 其他小程序框架...
}
```

**支持的框架与命令**

| 框架 | 栈 ID | dev_command | build_command | 检测文件 |
|------|-------|------------|--------------|---------|
| 原生 | `wechat-native` | `npm run dev:weixin` | `npm run build:weixin` | project.config.json, app.json, pages/*.wxml |
| Taro | `wechat-taro` | `npm run dev:weixin` | `npm run build:weixin` | taro.config.js, @tarojs/cli |
| uni-app | `wechat-uniapp` | `npm run dev:mp-weixin` | `npm run build:mp-weixin` | uni.config.js, pages.json |
| mpx | `wechat-mpx` | `npm run dev` | `npm run build` | mpx.config.js |

**依赖处理与预览**

- 包管理器自适应：同前端（npm/pnpm/yarn）
- 依赖缓存：`node_modules`（同前端）
- **预览机制**（区别于 Web iframe）：
  - 方案 A（推荐）：集成微信开发者工具 CLI（`wechat-devtools`）
    ```bash
    # 代码编译输出到 dist/mp-weixin 或 dist/mp
    npm run build:weixin
    # 开发者工具打开项目，自动登录、预览二维码
    wechat-devtools open --project-path ./dist/mp-weixin
    ```
  - 方案 B（轻量）：输出编译产物路径，前端展示提示 + 手动扫码
    - 编译输出目录路径
    - 生成预览 URL（需小程序后台配置）
    - 前端展示二维码供手动扫描

- **预览日志**：同 cr_preview.rs（AppEvent::PreviewLog）

**AI 代码生成适配**

- 编码 Agent Prompt 注入：
  ```
  # 微信小程序 (Taro 框架)
  - 文件结构：pages/{pageName}/index.tsx → index.scss + index.config.ts
  - 页面框架：useLoad + useState + Taro API（导航、存储、网络）
  - 样式：scss module，变量引用 src/styles/variables.scss
  - 调用后端：Taro.request() → 后端 API，自动注入 token（via Taro.getStorage）
  - 禁止：document.* / window.* / fetch / 浏览器 API
  ```

- 分析阶段（analysis.rs）：
  - 栈检测后自动识别小程序框架，记入 issue_analyses.stack_id
  - scope.target_files = ["pages/**/*.tsx", "pages/**/*.scss", "components/**/*.tsx"]

- 代码执行（execution.rs）：
  - 编码前：`npm install` + `npm run build` 验证编译通过
  - 编译检查：输出到 dist/mp-weixin，无 error
  - 测试钩子：`npm run test`（如 jest/vitest 配置）

**迁移与数据模型**

新增迁移 `00NN_wechat_mini_app_detection.sql`：
```sql
-- 扩展 code_agents 表的预训练 Prompt
-- wechat-native / wechat-taro / wechat-uniapp 已预设系统 Prompt 到 roles.rs

-- 扩展 project_specs 表的建议内容（可选自动生成）
-- 示例：「小程序 API 调用规范」、「页面路由配置」

-- 扩展 dev_servers 表的预览驱动选项
ALTER TABLE dev_servers ADD COLUMN weixin_preview_mode TEXT CHECK(weixin_preview_mode IN ('cli', 'manual', 'auto'));
```

---

### Phase 2: 前端品类优化（优先级：中）

#### 2.1 增强现有框架支持

**新增框架识别**

| 框架 | 关键依赖 | 影响 |
|------|---------|------|
| Remix | @remix-run/react | 同 React，新增 build:remix 脚本识别 |
| Astro | astro | dev_command 补充 `astro dev` 脚本检测 |
| Qwik | @builder.io/qwik | 同 React，但 build 输出目录为 dist |
| SolidJS | solid-js | 同 React，脚本检测 |
| Nuxt 3+ | nuxt | 已支持，强化版本检测（2 vs 3 API 差异） |

**代码更新**（core/stack.rs）

```rust
fn frontend_framework(deps: &[String]) -> &'static str {
    let has = |needle: &str| deps.iter().any(|d| d == needle);
    if has("next") { "next" }
    else if has("nuxt") { "nuxt" }
    else if has("@remix-run/react") { "remix" }
    else if has("astro") { "astro" }
    else if has("@builder.io/qwik") { "qwik" }
    else if has("solid-js") { "solid" }
    // ... 原有框架
}
```

#### 2.2 包管理器扩展

- ✅ npm / pnpm / yarn / bun （已完全支持）
- 新增：**Deno**（应对 Deno Fresh / Deno Deploy）
  - 检测：`deno.json` / `import_map.json`
  - 命令：`deno run --allow-net --allow-read=. dev.ts`

#### 2.3 后台管理前端专项

内置 Prompt 优化（roles.rs）：

```
# 后台管理前端（React/Vue）
## 常见需求模式
- 列表页：CRUD 操作、分页、搜索、批量操作、权限检查
- 表单页：验证、草稿保存、回源填充、动态字段
- 仪表板：KPI 卡片、图表（ECharts / Recharts）、实时推送
- 权限：@RequireAdmin / v-if="user.canDelete"

## 依赖约定
- UI 库：antd / element-ui / arco-design（检测 + prompt 明确）
- 表格：ag-grid / react-table / vue-datatable
- 状态管理：Redux / Vuex / Pinia
- 请求：axios / ky / fetch（via service 层）

## 代码结构
- pages/{pageName}/index.tsx → components + services 分离
- services 做 API 调用，pages 做状态 + 渲染
- 每个表单/列表配套 types.ts（接口定义）
```

---

### Phase 3: 后端品类优化（优先级：中）

#### 3.1 Java 生态扩展

**框架强化**

| 框架 | 检测 | dev_command | 支持度 |
|------|------|------------|--------|
| Spring Boot | ✅ | ✅ | 完整 |
| Quarkus | pom.xml/gradle | `./mvnw quarkus:dev` | 新增 |
| Micronaut | gradle | `./gradlew run` | 新增 |
| Ktor | gradle | `./gradlew run` | 新增 |
| Spring Cloud | ✅（同 Spring Boot） | - | 后续微服务编排 |

**安全扫描器**

- ✅ OWASP Dependency-Check（现有）
- 新增：`snyk test`（可选依赖）

#### 3.2 Go 生态扩展

**框架强化**

| 框架 | 检测 | 支持度 |
|------|------|--------|
| Gin | ✅ | 完整 |
| Echo | ✅ | 完整 |
| Fiber | ✅ | 完整 |
| Chi | go.mod | 新增 |
| Iris | go.mod | 新增 |
| Beego | go.mod | 新增 |

**依赖缓存**

- 当前：无（go.mod 和下载的包在 GOMODCACHE）
- 改进：检测 vendor/ 存在时，软链到 worktree（加快编译）

#### 3.3 Python 生态扩展

**框架强化**

| 框架 | 检测 | 支持度 |
|------|------|--------|
| FastAPI | ✅ | 完整 |
| Django | ✅ | 完整 |
| Flask | ✅ | 完整 |
| Starlette | pyproject.toml | 新增 |
| Sanic | requirements.txt | 新增 |
| Bottle | requirements.txt | 新增 |

**包管理扩展**

- ✅ poetry / pip / pipenv
- 新增：**uv**（超快 Python 包管理，Ruff 作者）
  - 检测：uv.lock / pyproject.toml 中 `[build-system] requires = ["uv"]`
  - 命令：`uv run` 前缀替代 `poetry run`

#### 3.4 Node.js 后端扩展

**框架强化**

| 框架 | 检测 | 支持度 |
|------|------|--------|
| Express | ✅ | 完整 |
| NestJS | ✅ | 完整 |
| Fastify | ✅ | 完整 |
| Koa | ✅ | 完整 |
| Hapi | ✅ | 完整 |
| Remix Server | @remix-run/express | 新增 |
| SvelteKit Server | @sveltejs/kit | 新增 |
| Nuxt Server Mode | nuxt | 新增 |

---

### Phase 4: 其他品类（优先级：低，未来方向）

#### 4.1 移动应用框架（技术预研）

| 平台 | 框架 | 检测文件 | 状态 |
|------|------|---------|------|
| iOS | Swift (Xcode) | Package.swift / .pbxproj | 预研 |
| Android | Kotlin (Android Studio) | build.gradle / AndroidManifest.xml | 预研 |
| React Native | React Native CLI | app.json (Expo) | 预研 |
| Flutter | Flutter SDK | pubspec.yaml | 预研 |

**现阶段方案**：认可但不自动检测，用户手动指定栈类型。

#### 4.2 更多小程序平台（后续）

- 抖音小程序（框架：同 Taro / 原生）
- 支付宝小程序（框架：同 Taro / 原生）
- 百度智能小程序
- 快手小程序

**实施方式**：Phase 1 完成微信小程序通用架构后，扩展框架识别即可。

---

## 🔧 核心实现清单

### 文件改动点

#### 1. 栈检测模块（core/stack.rs）

| 函数 | 改动 | 优先级 |
|------|------|--------|
| `detect_wechat_mini_app()` | 新增 | P1 |
| `detect_node()` → `frontend_framework()` | 增加框架 | P2 |
| `detect_node()` → `is_node_backend()` | 增强识别 | P2 |
| `detect_java()` | 新框架支持 | P3 |
| `detect_go()` | 新框架支持 | P3 |
| `detect_python()` | 新框架 + uv 支持 | P3 |
| `detect_stacks()` | 顺序调整 | P1 |
| 单元测试 | 补齐新框架 | 每个 PR |

#### 2. AI 角色与 Prompt（agents/roles.rs / llm.rs）

新增系统 Prompt 模板：

```
- wechat_native_prompt
- wechat_taro_prompt
- wechat_uniapp_prompt
- admin_frontend_react_prompt（强化）
- admin_frontend_vue_prompt（强化）
- java_microservice_prompt（新增）
```

映射到 `agents` 表（迁移中注入种子数据）。

#### 3. 预览与开发服务器

**dev_servers.rs** 新增

```rust
pub struct DevServerConfig {
    // ... 现有
    pub weixin_preview_mode: Option<String>, // "cli" | "manual" | "auto"
    pub weixin_devtools_path: Option<String>, // 自定义 wechat-devtools 路径
}
```

**cr_preview.rs** 新增预览适配

```rust
match primary_stack.id.as_str() {
    "wechat-native" | "wechat-taro" | "wechat-uniapp" => {
        // 特化预览：编译 → 二维码 / CLI 打开
        preview_wechat_mini_app(stack, &config).await
    }
    _ => { /* 现有逻辑 */ }
}
```

#### 4. 迁移文件（migrations/）

```
- 00NN_extend_dev_servers_for_weixin.sql
  └─ ALTER TABLE dev_servers ADD weixin_preview_mode ...
  
- 00NN_inject_wechat_mini_app_roles.sql
  └─ INSERT INTO agents (role_type, system_kind, role_name, system_prompt, visible_in_chat, ...)
     VALUES ('developer', 'wechat_taro', 'Taro Expert', '<prompt>', 1, ...)
```

#### 5. 前端页面（Settings / Projects）

**Settings.tsx** 新增选项

```tsx
<div className="field">
  <label className="field-label">默认小程序框架</label>
  <select className="proj-select" value={wechatFramework} onChange={...}>
    <option value="native">微信原生</option>
    <option value="taro">Taro</option>
    <option value="uniapp">uni-app</option>
  </select>
</div>
```

**Projects.tsx** 栈检测结果展示

```tsx
{stacks.map(s => (
  <div key={s.id} className="chip ember">
    {s.id} ({s.language}, {s.framework})
  </div>
))}
```

---

## 📋 迭代计划（推荐顺序）

### Sprint 1: 微信小程序基础（2 周）

- [x] `detect_wechat_mini_app()` 实现（原生 + Taro 框架）
- [x] 单元测试覆盖微信小程序检测
- [x] 系统 Prompt 注入（wechat_taro_prompt）
- [x] 迁移：agents 表种子数据
- [x] 简化预览：编译 → 输出 dist 路径

**输出**：能检测并生成 Taro 小程序代码；预览为编译产物提示

---

### Sprint 2: 微信小程序预览增强（1.5 周）

- [ ] 集成 `wechat-devtools` CLI
- [ ] 预览 QR 码生成（via weixin API）
- [ ] dev_servers 表扩展（weixin_preview_mode）
- [ ] cr_preview.rs 小程序专项预览流程
- [ ] 前端 LiveLogModal 适配小程序编译日志

**输出**：一键启动开发者工具或扫码预览

---

### Sprint 3: 前端品类优化（1 周）

- [ ] 框架检测增强（Remix / Astro / Qwik 等）
- [ ] Deno 支持
- [ ] 单元测试补齐
- [ ] 系统 Prompt 优化（后台管理前端专项）

**输出**：支持主流新框架，后台前端 Prompt 质量提升

---

### Sprint 4: 后端品类优化（1.5 周）

- [ ] Java：Quarkus / Micronaut 支持
- [ ] Go：Chi / Iris 支持 + vendor/ 软链
- [ ] Python：uv 支持 + Starlette / Sanic 框架
- [ ] Node：Remix / SvelteKit 后端模式识别
- [ ] 单元测试补齐

**输出**：后端框架库完整，质量闸口更强

---

### Sprint 5: 内容与文档（1 周）

- [ ] 更新 CLAUDE.md 新栈信息
- [ ] 各品类对应的编码规范指南（.autoforge/specs）
- [ ] 迁移清单 + Rollback 方案
- [ ] 团队培训文档

**输出**：文档齐全，开发者无障碍上手

---

## ⚙️ 验收标准

### 单元测试覆盖

```
✅ detect_wechat_mini_app() 检测原生、Taro、uni-app
✅ 多栈项目返回正确优先级排序
✅ 前端框架检测无重复（Remix / Next 识别正确）
✅ dep_cache_dirs 包括 node_modules + vendor
```

### 集成测试

```
✅ 创建 Taro 项目 → 检测 → 提交需求 → AI 生成代码 → 编译通过
✅ 后台管理前端项目 → 适当 Prompt 注入 → 代码质量提升
✅ 预览能启动编译 + 展示日志
```

### 代码审核

```
✅ 无 Tauri 类型污染（stack.rs 纯 Rust）
✅ Prompt 与规范一致（roles.rs）
✅ 迁移可回滚（checksum 正确）
✅ 性能无退化（detect_stacks 首次 < 50ms）
```

---

## 🚧 风险与缓解

| 风险 | 影响 | 缓解方案 |
|------|------|---------|
| 微信小程序编译需要登录 | 自动化预览困难 | 二维码 + 手动扫描（fallback） |
| 框架库膨胀→编码 Prompt 复杂 | AI 生成质量下降 | 角色化 Prompt（专用 role）+ 用户选择确认 |
| 后端新框架命令差异大 | 通用 {port} 占位失效 | 框架特化的命令模板 + fallback 原生命令 |
| 依赖缓存策略（软链）跨平台 | Windows 开发困难 | 文档标注 Unix only，CI 在 Linux 验证 |

---

## 📚 参考资源

- [微信小程序官方文档](https://developers.weixin.qq.com/miniprogram/dev/)
- [Taro 文档](https://taro.jd.com/)
- [uni-app 文档](https://uniapp.dcloud.io/)
- [wechat-devtools CLI](https://github.com/nwutils/wechat-devtools)
- Rust 栈：[sqlx](https://github.com/launchbadge/sqlx), [serde](https://serde.rs/)

---

## ✨ 后续优化方向

1. **多小程序平台**：框架层面统一，平台层面差异化
2. **Mobile 生态**（预研）：React Native / Flutter 识别与代码生成
3. **编码 Agent 多模型**：不同品类绑定最优模型（快速 vs 强大）
4. **预览环境隔离**：Podman / Docker 沙箱化编译（跨平台安全）
5. **增量编译缓存**：Git worktree 内 Rust/Go/Node 增量编译加速
