import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import type { PrinterProfile } from '../api/types';
import { filterPrinters, foundPrintersMessage, groupPrinters } from '../utils/printers';
import Icon from './Icon';

/** Значение «Моего принтера нет в списке»: дальше форма спрашивает размеры стола. */
export const CUSTOM_PRINTER = 'custom';
const CUSTOM_LABEL = 'Моего принтера нет в списке';
/** Высота липкого заголовка производителя и нижний отступ списка (workspace.css .printer-*). */
const STICKY_HEAD_PX = 30;
const LIST_PAD_PX = 6;

interface PrinterPickerProps {
  profiles: PrinterProfile[];
  /** id профиля, CUSTOM_PRINTER или null (ещё не выбран). */
  value: string | typeof CUSTOM_PRINTER | null;
  onChange: (value: string) => void;
  /** Принтер определён по 3MF: только показ, сервер примет лишь его. */
  locked?: boolean;
  disabled?: boolean;
  /** id поля ввода — для <label htmlFor>. Не оборачивать пикер в <label>: клик по опции ушёл бы в input. */
  id?: string;
  placeholder?: string;
  /**
   * Только «Моего принтера нет в списке»: 3MF неизвестного принтера пакер примет лишь со своим
   * столом (он сверяет printer_model проекта с printer_name профиля), каталог ему не подходит.
   */
  customOnly?: boolean;
}

/**
 * Поиск принтера по каталогу: поле ввода (role=combobox) и всплывающий listbox, сгруппированный
 * по производителю (заголовки липнут к верху списка). Фокус всегда остаётся в поле: опции не
 * фокусируются, активная показывается через aria-activedescendant. Последняя опция фиксирована —
 * «Моего принтера нет в списке», она не фильтруется поиском.
 */
export default function PrinterPicker({
  profiles,
  value,
  onChange,
  locked = false,
  disabled = false,
  id,
  placeholder = 'Выберите принтер',
  customOnly = false,
}: PrinterPickerProps) {
  const autoId = useId();
  const inputId = id ?? `${autoId}-input`;
  const listId = `${autoId}-list`;
  const optionId = (index: number) => `${autoId}-opt-${index}`;

  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(-1);
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  /** Список только что открыт: выбранную опцию прокрутить к центру, а не к краю. */
  const justOpened = useRef(false);

  const groups = useMemo(
    () => (customOnly ? [] : groupPrinters(filterPrinters(profiles, query))),
    [customOnly, profiles, query],
  );
  // Плоский порядок опций для стрелок: заголовки производителей в него не входят.
  const values = useMemo(
    () => [...groups.flatMap((g) => g.items.map((p) => p.id)), CUSTOM_PRINTER],
    [groups],
  );
  const indexOf = useMemo(() => new Map(values.map((v, i) => [v, i])), [values]);

  const selectedLabel =
    value === CUSTOM_PRINTER ? CUSTOM_LABEL : (profiles.find((p) => p.id === value)?.name ?? '');

  const openList = () => {
    if (open) return;
    justOpened.current = true;
    setOpen(true);
    setQuery('');
    const all = customOnly ? [] : groupPrinters(profiles).flatMap((g) => g.items.map((p) => p.id));
    const index = value === CUSTOM_PRINTER ? all.length : all.indexOf(value ?? '');
    setActive(index >= 0 ? index : 0);
  };

  const close = () => {
    setOpen(false);
    setQuery('');
    setActive(-1);
  };

  const select = (next: string) => {
    onChange(next);
    close();
  };

  // Клик мимо пикера закрывает список (blur это тоже ловит, но не клик по самому полю-соседу без фокуса).
  useEffect(() => {
    if (!open) return undefined;
    const onMouseDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) close();
    };
    document.addEventListener('mousedown', onMouseDown);
    return () => document.removeEventListener('mousedown', onMouseDown);
  }, [open]);

  // Активная опция всегда видна: при открытии выбранная по центру списка, дальше — ближайшим краем
  // с запасом под липкий заголовок. Прокручиваем только список: scrollIntoView двигал бы и страницу.
  useEffect(() => {
    if (!open || active < 0) return;
    const list = document.getElementById(listId);
    const el = document.getElementById(optionId(active));
    if (!list || !el) return;
    const top = el.offsetTop;
    const bottom = top + el.offsetHeight;
    if (justOpened.current) {
      justOpened.current = false;
      list.scrollTop = top - (list.clientHeight - el.offsetHeight) / 2;
    } else if (top - STICKY_HEAD_PX < list.scrollTop) {
      list.scrollTop = top - STICKY_HEAD_PX;
    } else if (bottom > list.scrollTop + list.clientHeight) {
      list.scrollTop = bottom - list.clientHeight + LIST_PAD_PX;
    }
    // listId/optionId зависят только от autoId — стабильны.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, active]);

  if (locked) {
    return (
      <div className="printer-picker">
        <output id={inputId} className="input printer-picker-locked">
          <span className="printer-picker-value">{selectedLabel || '—'}</span>
          <span className="printer-picker-note">определён по файлу</span>
        </output>
      </div>
    );
  }

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const last = values.length - 1;
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        if (!open) openList();
        else setActive((i) => Math.min(last, i + 1));
        break;
      case 'ArrowUp':
        e.preventDefault();
        if (!open) openList();
        else setActive((i) => Math.max(0, i - 1));
        break;
      case 'Home':
        if (!open) return;
        e.preventDefault();
        setActive(0);
        break;
      case 'End':
        if (!open) return;
        e.preventDefault();
        setActive(last);
        break;
      case 'Enter':
        // Закрытый список — обычный Enter формы; открытый — выбор, форма не отправляется.
        if (!open) return;
        e.preventDefault();
        if (active >= 0 && active <= last) select(values[active]);
        break;
      case 'Escape':
        if (!open) return;
        // Только закрыть список: не даём Esc всплыть к модалке/странице. React-овский stopPropagation
        // не останавливает нативные слушатели document (React слушает на корне, событие идёт дальше
        // вверх), поэтому — и нативное событие. Слушатели в фазе захвата это не остановит.
        e.preventDefault();
        e.stopPropagation();
        e.nativeEvent.stopImmediatePropagation();
        close();
        break;
      case 'Tab':
        if (open) close();
        break;
    }
  };

  const option = (optionValue: string, label: string) => {
    const index = indexOf.get(optionValue) ?? -1;
    const selected = optionValue === value;
    return (
      <div
        key={optionValue}
        id={optionId(index)}
        role="option"
        aria-selected={selected}
        className={`printer-option${index === active ? ' active' : ''}${optionValue === CUSTOM_PRINTER ? ' custom' : ''}`}
        onMouseMove={() => {
          if (index !== active) setActive(index);
        }}
        onClick={() => select(optionValue)}
      >
        <span className="printer-option-label">{label}</span>
        {selected ? <Icon name="check" /> : null}
      </div>
    );
  };

  const empty = groups.length === 0 && !customOnly;
  const found = groups.reduce((sum, g) => sum + g.items.length, 0);

  return (
    <div ref={rootRef} className={`printer-picker${open ? ' open' : ''}`}>
      <input
        ref={inputRef}
        id={inputId}
        className="input printer-picker-input"
        type="text"
        role="combobox"
        aria-expanded={open}
        // Список рендерится только открытым: ссылка на несуществующий id — ошибка ARIA.
        aria-controls={open ? listId : undefined}
        aria-autocomplete="list"
        aria-activedescendant={open && active >= 0 ? optionId(active) : undefined}
        autoComplete="off"
        spellCheck={false}
        disabled={disabled}
        placeholder={open ? selectedLabel || 'Поиск: производитель или модель' : placeholder}
        value={open ? query : selectedLabel}
        onChange={(e) => {
          if (!open) openList();
          setQuery(e.target.value);
          setActive(0);
        }}
        onClick={() => (open ? close() : openList())}
        onKeyDown={onKeyDown}
        onBlur={close}
      />
      {/* Число найденных при поиске — диктору (сам список о фильтрации не сообщает). */}
      <div role="status" aria-live="polite" className="visually-hidden">
        {open && query.trim() !== '' && !customOnly ? foundPrintersMessage(found) : ''}
      </div>
      <button
        type="button"
        className="printer-picker-toggle"
        tabIndex={-1}
        aria-hidden="true"
        disabled={disabled}
        // Без потери фокуса поля: иначе blur закрыл бы список раньше клика.
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => {
          inputRef.current?.focus();
          if (open) close();
          else openList();
        }}
      >
        <Icon name="chevron-down" />
      </button>
      {open ? (
        <div
          id={listId}
          role="listbox"
          aria-label="Принтеры"
          className="printer-list"
          // Клик по списку (и по его полосе прокрутки) не уводит фокус из поля.
          onMouseDown={(e) => e.preventDefault()}
        >
          {groups.map((group, groupIndex) => {
            // id по индексу: имя производителя с пробелами не годится для aria-labelledby (список id).
            const headerId = `${autoId}-vendor-${groupIndex}`;
            return (
              <div key={group.vendor} role="group" aria-labelledby={headerId} className="printer-group">
                <div id={headerId} role="presentation" className="printer-group-head">
                  {group.vendor}
                </div>
                {group.items.map((p) => option(p.id, p.name))}
              </div>
            );
          })}
          {empty ? (
            <div role="presentation" className="printer-list-empty">
              Ничего не найдено
            </div>
          ) : null}
          {customOnly ? null : <div role="presentation" className="printer-list-sep" />}
          {option(CUSTOM_PRINTER, CUSTOM_LABEL)}
        </div>
      ) : null}
    </div>
  );
}
