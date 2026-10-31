import { useEffect, useRef, useState } from 'react';
import type { MouseEvent } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { serverApi } from '@/api/http';
import Icon from '@/components/Icon';
import { useLinkHandler } from '@/hooks/useLinkHandler';
import type { Session } from '@/pages/ServerPages';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';

/** «petr» → «PE», «petr.popov» → «PP». */
export function initialsOf(login: string): string {
  const words = login.trim().split(/[\s._-]+/).filter(Boolean);
  return (words.length >= 2 ? words[0][0] + words[1][0] : login.slice(0, 2)).toUpperCase();
}

/**
 * Справа в шапке. Гость: «Войти» + меню (гостевая сессия, Мои задачи, Создать аккаунт).
 * Пользователь: аватар-меню (Мои задачи, Администрирование для админа, Выйти).
 * Сброса пароля здесь нет — только на /login. Закрытие: клик вне, Escape (фокус на триггер).
 */
export default function AccountMenu({ user }: { user?: Session }) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const onLink = useLinkHandler();
  const navigate = useUiStore((s) => s.navigate);
  const queryClient = useQueryClient();

  useEffect(() => {
    if (!open) return;
    const onPointer = (e: PointerEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener('pointerdown', onPointer);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onPointer);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const guest = !user || user.kind === 'GUEST';
  const go = (event: MouseEvent<HTMLAnchorElement>) => {
    setOpen(false);
    onLink(event);
  };
  const logout = async () => {
    setOpen(false);
    try {
      await serverApi.post('/auth/logout');
      await queryClient.invalidateQueries({ queryKey: ['session'] });
      navigate('/pack');
    } catch (e) {
      pushToast(e instanceof Error ? e.message : 'Не удалось выйти', 'error');
    }
  };

  return (
    <>
      {guest && (
        <a className="btn" href="/login" onClick={onLink}>
          Войти
        </a>
      )}
      <div className="menu-wrap" ref={wrapRef}>
        <button
          ref={triggerRef}
          type="button"
          className={guest ? 'icon-btn' : 'avatar'}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label={guest ? 'Меню' : `Аккаунт: ${user.login ?? ''}`}
          title={guest ? 'Меню' : (user.login ?? '')}
          onClick={() => setOpen((v) => !v)}
        >
          {guest ? <Icon name="menu" /> : initialsOf(user.login ?? '?')}
        </button>
        {open && (
          <div className="menu" role="menu">
            <div className="menu-head">
              <b>{guest ? 'Гостевая сессия' : user.login}</b>
              <span>{guest ? 'результаты хранятся 24 часа' : user.role === 'ADMIN' ? 'администратор' : 'аккаунт'}</span>
            </div>
            <a className="menu-item" role="menuitem" href="/jobs" onClick={go}>
              <Icon name="list" />
              Мои задачи
            </a>
            {guest ? (
              <a className="menu-item" role="menuitem" href="/login/register" onClick={go}>
                <Icon name="user" />
                Создать аккаунт
              </a>
            ) : (
              <>
                {user.role === 'ADMIN' && (
                  <a className="menu-item" role="menuitem" href="/admin" onClick={go}>
                    <Icon name="shield" />
                    Администрирование
                  </a>
                )}
                <div className="menu-sep" role="separator" />
                <button type="button" className="menu-item" role="menuitem" onClick={() => void logout()}>
                  <Icon name="logout" />
                  Выйти
                </button>
              </>
            )}
          </div>
        )}
      </div>
    </>
  );
}
