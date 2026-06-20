import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
// 霞鹜文楷 LXGW WenKai（lxgw/LxgwWenkai）本地 webfont：阅读模式/报告正文字体。
// 仅引入 regular(400)+bold(700) 两种字重（report 用 700），woff2 由 Vite 本地打包，无需 CDN。
import 'lxgw-wenkai-webfont/lxgwwenkai-regular.css';
import 'lxgw-wenkai-webfont/lxgwwenkai-bold.css';
import './index.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
