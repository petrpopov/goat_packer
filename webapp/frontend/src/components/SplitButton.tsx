import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import Icon, { type IconName } from './Icon';

export type MenuItem =
  { label: string; icon: IconName; onSelect: () => void; note?: string; disabled?: boolean } | 'separator';

interface SplitButtonProps {
  /** Главное действие (оранжевая кнопка). */
  main: ReactNode;
  items: MenuItem[];
}

/**
 * Главная кнопка + стрелка «ещё»: одно оранжевое действие, остальные — в меню. Меню закрывается
 * кликом мимо, Escape и выбором пункта; стрелки ↑/↓ ходят по пунктам.
 */
export default function SplitButton({ main, items }: SplitButtonProps) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false);
      if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
      const buttons = [...(menu.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])];
      const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
      const next = event.key === 'ArrowDown' ? at + 1 : at - 1;
      buttons[(next + buttons.length) % buttons.length]?.focus();
      event.preventDefault();
    };
    document.addEventListener('pointerdown', outside);
    document.addEventListener('keydown', key);
    menu.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
    return () => {
      document.removeEventListener('pointerdown', outside);
      document.removeEventListener('keydown', key);
    };
  }, [open]);
  return (
    <div className="split" ref={root}>
      {main}
      <button
        type="button"
        className="btn btn-primary btn-lg"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Ещё действия"
        data-tip={open ? undefined : 'Ещё действия'}
        data-tip-align="end"
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="chevron-down" />
      </button>
      {open ? (
        <div className="menu" role="menu" ref={menu}>
          {items.map((item, index) =>
            item === 'separator' ? (
              <hr key={`sep-${index}`} />
            ) : (
              <button
                key={item.label}
                type="button"
                role="menuitem"
                disabled={item.disabled}
                onClick={() => {
                  setOpen(false);
                  item.onSelect();
                }}
              >
                <Icon name={item.icon} />
                {item.label}
                {item.note ? <small>{item.note}</small> : null}
              </button>
            )
          )}
        </div>
      ) : null}
    </div>
  );
}
