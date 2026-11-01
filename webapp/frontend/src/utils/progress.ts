/** Этапы упаковки для шкалы и строки статуса (протокол stage-событий colored_3mf_packer). */

import type { JobStage, PackStageId } from '../api/types';

/** Канонический порядок этапов; необязательные — те, в которые пакер заходит не всегда. */
const ORDER: { id: PackStageId; label: string; optional: boolean }[] = [
  { id: 'probe_slice', label: 'Пробная нарезка 1 объекта', optional: false },
  { id: 'packing', label: 'Раскладка', optional: false },
  { id: 'sample_slice', label: 'Пробная нарезка нескольких объектов', optional: true },
  { id: 'assembling', label: 'Сборка проекта', optional: false },
  { id: 'final_slice', label: 'Нарезка итоговой пластины', optional: false },
  { id: 'gcode_check', label: 'Проверка поддержек по слоям', optional: true },
  { id: 'repacking', label: 'Перепаковка', optional: true },
  { id: 'window_slice', label: 'Подбор под окно времени', optional: true },
  { id: 'writing', label: 'Запись результата', optional: false },
];

/** «1 объект», «3 объекта», «37 объектов». */
export function objectsText(count: number): string {
  const mod100 = count % 100;
  const mod10 = count % 10;
  const word =
    mod100 >= 11 && mod100 <= 14
      ? 'объектов'
      : mod10 === 1
        ? 'объект'
        : mod10 >= 2 && mod10 <= 4
          ? 'объекта'
          : 'объектов';
  return `${count} ${word}`;
}

/** Длительность: «45 с», «2 мин 05 с», «1 ч 03 мин». */
export function formatDuration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  if (total < 60) return `${total} с`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60) return `${minutes} мин ${String(total % 60).padStart(2, '0')} с`;
  return `${Math.floor(minutes / 60)} ч ${String(minutes % 60).padStart(2, '0')} мин`;
}

/** Оставшееся время этапа: «осталось ≈ 3 мин»; null — у пакера нет оценки. */
export function remainingText(elapsedSeconds: number, expectedSeconds: number | null): string | null {
  if (expectedSeconds == null || !Number.isFinite(expectedSeconds) || expectedSeconds <= 0) return null;
  const left = expectedSeconds - elapsedSeconds;
  // Оценка пакера грубая: перебор до полутора оценок — ещё норма, «< 1 мин», а не тревога.
  if (left <= 0) return elapsedSeconds < expectedSeconds * 1.5 ? 'осталось < 1 мин' : 'дольше обычного';
  if (left < 60) return 'осталось < 1 мин';
  return `осталось ≈ ${Math.ceil(left / 60)} мин`;
}

/** Сколько этап идёт сейчас: серверное значение + время, прошедшее с получения ответа. */
export function stageElapsed(stage: JobStage, receivedAtMs: number | undefined, nowMs: number): number {
  const sinceReceived = receivedAtMs == null ? 0 : Math.max(0, nowMs - receivedAtMs) / 1000;
  return stage.elapsedSeconds + sinceReceived;
}

/** Строка времени этапа: «идёт 2 мин 05 с · осталось ≈ 3 мин». */
export function stageTimeLine(elapsedSeconds: number, expectedSeconds: number | null): string {
  const remaining = remainingText(elapsedSeconds, expectedSeconds);
  return `идёт ${formatDuration(elapsedSeconds)}${remaining ? ` · ${remaining}` : ''}`;
}

/**
 * Похоже на JSON-объект — такое пользователю не показываем (страховка поверх бэкенда). Только
 * «{»: строки планировщика начинаются с «[Принтер]» и должны показываться.
 */
export function looksLikeJson(text: string | null | undefined): boolean {
  return (text?.trim() ?? '').startsWith('{');
}

/** Полосы процента этапов — те же, что у бэкенда (ProgressTracker.PackStage): по ним срез в сцене. */
const BANDS: Partial<Record<PackStageId, [number, number]>> = {
  probe_slice: [1, 15],
  final_slice: [33, 78],
};

/** Подписи повторного захода: после перепаковки пакер снова собирает, режет и проверяет. */
const REPEAT_LABEL: Partial<Record<PackStageId, string>> = {
  assembling: 'Повторная сборка',
  final_slice: 'Повторная нарезка',
  gcode_check: 'Повторная проверка поддержек',
};

/** Высота плоскости среза в сцене как доля высоты модели (1 — модель целиком). */
export function sceneClip(stageId: string, percent: number): number {
  if (stageId !== 'probe_slice' && stageId !== 'final_slice') return 1;
  const [start, end] = BANDS[stageId]!;
  return Math.min(1, Math.max(0, (percent - start) / (end - start)));
}

/** Строка статуса: подпись текущего этапа и «этап N из M» (M растёт, если понадобились необязательные). */
export interface StripStage {
  label: string;
  step: number;
  steps: number;
}

/** Пункт списка этапов в панели «этапы»: пройденные, текущий и ещё обязательные. */
export interface StageItem {
  key: string;
  label: string;
  state: 'done' | 'current' | 'pending';
}

function stagePlan(stage: JobStage) {
  const visited = stage.visited.length ? stage.visited : [stage.id];
  const currentIndex = ORDER.findIndex((step) => step.id === stage.id);
  const furthest = ORDER.reduce(
    (max, step, index) => (step.id !== stage.id && visited.includes(step.id) ? Math.max(max, index) : max),
    -1
  );
  // Повторный заход: после перепаковки пакер снова собирает, режет и проверяет.
  const repeat = (currentIndex >= 0 && furthest > currentIndex) || stage.attempt > 1;
  const from = Math.max(currentIndex, furthest);
  const remaining = ORDER.filter((step, index) => index > from && !step.optional && !visited.includes(step.id));
  const label =
    (repeat ? REPEAT_LABEL[stage.id as PackStageId] : undefined) ??
    ORDER.find((step) => step.id === stage.id)?.label ??
    'Упаковка';
  return { visited, repeat, remaining, label };
}

export function stripStage(stage: JobStage): StripStage {
  const { visited, repeat, remaining, label } = stagePlan(stage);
  const step = visited.length + (repeat ? Math.max(1, stage.attempt - 1) : 0);
  return { label, step, steps: step + remaining.length };
}

export function stageItems(stage: JobStage): StageItem[] {
  const { visited, repeat, remaining, label } = stagePlan(stage);
  const labelOf = (id: string) => ORDER.find((step) => step.id === id)?.label ?? id;
  const items: StageItem[] = visited
    .filter((id) => repeat || id !== stage.id)
    .map((id) => ({ key: id, label: labelOf(id), state: 'done' }));
  items.push({ key: `${stage.id}:current`, label, state: 'current' });
  for (const step of remaining) items.push({ key: step.id, label: step.label, state: 'pending' });
  return items;
}

/** Строка панели «Инфо»: этап, его состояние и длительность, с (null — неизвестна). */
export interface StageRow {
  key: string;
  label: string;
  state: 'done' | 'current' | 'pending' | 'failed';
  seconds: number | null;
}

/**
 * Этапы с длительностями из истории заходов (бэкенд хранит её и после завершения задачи). running —
 * последний открытый заход идёт сейчас (его время — elapsedNow); у завершённой задачи открытый заход
 * закрывается её finishedAtMs, failed — на нём задача упала. Без истории (старые задачи) — прежний
 * список без времени.
 */
export function stageRows(
  stage: JobStage,
  {
    running,
    failed,
    elapsedNow,
    finishedAtMs,
  }: { running: boolean; failed: boolean; elapsedNow: number; finishedAtMs: number | null }
): StageRow[] {
  const history = stage.history ?? [];
  if (!history.length) return stageItems(stage).map((item) => ({ ...item, seconds: null }));
  const labelOf = (id: string) => ORDER.find((step) => step.id === id)?.label ?? id;
  const seen = new Set<string>();
  const rows: StageRow[] = history.map((span, index) => {
    const repeat = span.attempt > 1 || seen.has(span.id);
    seen.add(span.id);
    const last = index === history.length - 1;
    const open = span.endedAtMs == null;
    const seconds = !open
      ? (span.endedAtMs! - span.startedAtMs) / 1000
      : running && last
        ? elapsedNow
        : finishedAtMs != null
          ? (finishedAtMs - span.startedAtMs) / 1000
          : null;
    return {
      key: `${span.id}:${index}`,
      label: (repeat ? REPEAT_LABEL[span.id as PackStageId] : undefined) ?? labelOf(span.id),
      state: running && last ? 'current' : failed && last ? 'failed' : 'done',
      seconds: seconds == null ? null : Math.max(0, seconds),
    };
  });
  if (running)
    for (const step of stagePlan(stage).remaining)
      rows.push({ key: step.id, label: step.label, state: 'pending', seconds: null });
  return rows;
}
