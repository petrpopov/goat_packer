import { useEffect } from 'react';
import type { ReactNode } from 'react';
import Icon from './Icon';

/** Панель поверх сцены (этапы, файлы, ошибка): в потоке страницы её нет — стол не сдвигается. */
export default function ScenePopover({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [onClose]);
  return (
    <div className="scene-pop" role="dialog" aria-label={title}>
      <div className="scene-pop-head">
        <h3>{title}</h3>
        <button
          type="button"
          className="icon-btn"
          aria-label="Закрыть"
          data-tip="Закрыть"
          data-tip-align="end"
          onClick={onClose}
        >
          <Icon name="x" />
        </button>
      </div>
      {children}
    </div>
  );
}
