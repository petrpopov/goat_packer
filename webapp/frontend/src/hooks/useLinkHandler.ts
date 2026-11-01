import type { MouseEvent } from 'react';
import { useUiStore } from '@/store/useUiStore';

/**
 * Внутренние ссылки — обычные <a href>: средняя кнопка и «открыть в новой вкладке» работают,
 * обычный клик переходит без перезагрузки страницы.
 */
export function useLinkHandler() {
  const navigate = useUiStore((s) => s.navigate);
  return (event: MouseEvent<HTMLAnchorElement>) => {
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    )
      return;
    event.preventDefault();
    navigate(event.currentTarget.getAttribute('href') ?? '/');
  };
}
