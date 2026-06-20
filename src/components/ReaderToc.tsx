import { useEffect, useState, type RefObject } from 'react';

type Heading = { level: number; text: string; el: HTMLElement };

// 全屏阅读模式（会议室阅读模式 / 审计页报告全屏）共用的大纲目录。
// 与内嵌聊天的 docHeadings 解析不同：这里直接扫描已渲染容器内的 h1–h3
// DOM 节点，因此对「Markdown 气泡」和「静态报告（.report 的 <h2>）」一视同仁。
// 用 MutationObserver 跟随异步内容（分析结果/规格/代码高亮）变化自动重扫。
export function ReaderToc({ scrollRef, watch, className }: {
  /** 滚动容器（同时是 querySelector 根与点击跳转时的滚动父级）。 */
  scrollRef: RefObject<HTMLElement | null>;
  /** 内容来源标识：变化时强制重新挂载观察器并重扫。 */
  watch?: unknown;
  className?: string;
}) {
  const [items, setItems] = useState<Heading[]>([]);

  useEffect(() => {
    const root = scrollRef.current;
    if (!root) { setItems([]); return; }
    let raf = 0;
    const scan = () => {
      const hs = Array.from(root.querySelectorAll<HTMLElement>('h1, h2, h3'));
      const next: Heading[] = hs
        .map(el => ({ level: Number(el.tagName[1]) || 1, text: (el.textContent ?? '').trim(), el }))
        .filter(h => h.text);
      setItems(next);
    };
    scan();
    const obs = new MutationObserver(() => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(scan);
    });
    obs.observe(root, { childList: true, subtree: true, characterData: true });
    return () => { cancelAnimationFrame(raf); obs.disconnect(); };
  }, [scrollRef, watch]);

  // 少于 2 个标题时不值得占位
  if (items.length < 2) return null;

  return (
    <nav className={'doc-toc reader-toc' + (className ? ' ' + className : '')} aria-label="文档大纲">
      <div className="doc-toc-label">大纲</div>
      {items.map((h, i) => (
        <a
          key={i}
          className={`lvl-${h.level}`}
          title={h.text}
          onClick={() => h.el.scrollIntoView({ behavior: 'smooth', block: 'start' })}
        >
          {h.text}
        </a>
      ))}
    </nav>
  );
}
