# 编码规范

## Tauri 权限声明

每个 JS→Rust IPC 调用必须在 src-tauri/capabilities/main.json 中显式声明对应权限，缺失权限会导致运行时 not allowed 报错，不属于代码 bug。

---

## CSS 变量规范

前端样式只使用 src/index.css 中定义的 CSS 变量（如 var(--ember)、var(--bg-2)），禁止硬编码颜色值或字体尺寸。

---

## 禁用原生下拉

禁止使用 <select> 原生控件，统一采用 proj-select + mention-pop + mention-row 自定义下拉模式，参考 Audit.tsx 实现。

---

## 迁移文件不可变

src-tauri/migrations/ 下已有 SQL 迁移文件不可修改，新增数据模型只能创建序号递增的新文件（如 00NN_description.sql）。

---

## 新增 Command 流程

新增 Tauri command 需完成：commands/<module>.rs 编写、mod.rs 导出、lib.rs 注册、services/index.ts 封装，四步缺一不可。
