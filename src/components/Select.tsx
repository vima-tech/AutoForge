import { useState, useRef, useEffect, CSSProperties } from 'react';
import { createPortal } from 'react-dom';

export interface SelectOption {
  value: string;
  label: string;
}

interface SelectProps {
  value: string;
  onChange: (value: string) => void;
  options: SelectOption[];
  placeholder?: string;
  className?: string;
  style?: CSSProperties;
}

interface PopPos { top: number; left: number; width: number; flipUp: boolean; }

export default function Select({ value, onChange, options, placeholder = '请选择', className, style }: SelectProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<PopPos | null>(null);
  const triggerRef = useRef<HTMLDivElement>(null);

  const selected = options.find(o => o.value === value);

  const calcPos = () => {
    if (!triggerRef.current) return;
    const r = triggerRef.current.getBoundingClientRect();
    const popH = Math.min(options.length * 37 + 8, 240);
    const spaceBelow = window.innerHeight - r.bottom - 4;
    const flipUp = spaceBelow < popH && r.top > popH;
    setPos({
      left: r.left,
      width: r.width,
      top: flipUp ? r.top - 4 - popH : r.bottom + 4,
      flipUp,
    });
  };

  useEffect(() => {
    if (!open) { setPos(null); return; }
    calcPos();
    const close = () => setOpen(false);
    window.addEventListener('scroll', close, true);
    window.addEventListener('resize', close);
    return () => {
      window.removeEventListener('scroll', close, true);
      window.removeEventListener('resize', close);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (triggerRef.current && !triggerRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  const handleKey = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') setOpen(false);
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setOpen(o => !o); }
    if (e.key === 'ArrowDown' && !open) setOpen(true);
  };

  return (
    <div ref={triggerRef} className={`csel${className ? ' ' + className : ''}`} style={style}>
      <div
        className={`csel-trigger${open ? ' open' : ''}`}
        onClick={() => setOpen(o => !o)}
        onKeyDown={handleKey}
        tabIndex={0}
        role="combobox"
        aria-expanded={open}
        aria-haspopup="listbox"
      >
        <span className={selected ? 'csel-val' : 'csel-ph'}>
          {selected ? selected.label : placeholder}
        </span>
        <svg className={`csel-arr${open ? ' open' : ''}`} width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </div>

      {open && pos && createPortal(
        <div
          className={`csel-pop${pos.flipUp ? ' flip' : ''}`}
          role="listbox"
          style={{ position: 'fixed', top: pos.top, left: pos.left, width: pos.width }}
        >
          {options.map(opt => (
            <div
              key={opt.value}
              className={`csel-opt${opt.value === value ? ' active' : ''}`}
              role="option"
              aria-selected={opt.value === value}
              onMouseDown={e => { e.preventDefault(); onChange(opt.value); setOpen(false); }}
            >
              {opt.label}
            </div>
          ))}
        </div>,
        document.body
      )}
    </div>
  );
}
