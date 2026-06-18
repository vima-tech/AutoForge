use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProjectSpec {
    pub id: String,
    pub project_id: String,
    pub category: String,
    pub title: String,
    pub content: String,
    pub sort_order: i64,
    /// 规格来源：'db'(内容内联在 content) | 'file'(内容在磁盘 rel_path)
    #[serde(default)]
    pub source: String,
    /// file 源的仓库相对路径（db 源为空）
    #[serde(default)]
    pub rel_path: String,
    /// 清单/工具里展示的简短摘要（渐进读取的「声明」）
    #[serde(default)]
    pub description: String,
    /// 注入档位：'always'(全文常驻) | 'on_demand'(工具按需读) | 'off'(不暴露)
    #[serde(default)]
    pub injection: String,
    pub created_at: String,
    pub updated_at: String,
}
