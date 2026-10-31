/** Этапы упаковки для шкалы и строки статуса (протокол stage-событий colored_3mf_packer). */

import type { JobStage, PackStageId } from '../api/types';

/** Канонический порядок этапов; необязательные — те, в которые пакер заходит не всегда. */
const ORDER: { id: PackStageId; label: string; optional: boolean }[] = [
  { id: 'probe_slice', label: 'Пробная нарезка 1 объекта', optional: false },
  { id: 'packing', label: 'Раскладка', optional: false },
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
    mod100 >= 11 && mod100 <= 14 ? 'объектов' : mod10 === 1 ? 'объект' : mod10 >= 2 && mod10 <= 4 ? 'объекта' : 'объектов';
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
  if (left <= 0) return 'дольше обычного';
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

/** Полосы процента этапов — те же, что у бэкенда (ProgressTracker.PackStage). */
const BANDS: Record<PackStageId, [number, number]> = {
  probe_slice: [1, 15],
  packing: [15, 30],
  assembling: [30, 33],
  final_slice: [33, 78],
  gcode_check: [78, 82],
  repacking: [82, 86],
  window_slice: [86, 95],
  writing: [95, 99],
};

export type SegmentState = 'done' | 'current' | 'pending' | 'skipped';
export interface StageSegment {
  id: PackStageId;
  label: string;
  /** Доля ширины полосы = ширина полосы этапа в процентах. */
  weight: number;
  state: SegmentState;
  /** Заполнение текущего сегмента 0..1. */
  fill: number;
}

/**
 * Сегменты шкалы: все этапы по порядку; необязательные, пройденные без захода, — «skipped».
 * Посещённый этап — «done», даже если стоит после текущего: повторная нарезка после перепаковки
 * снова идёт через final_slice, а проверка и перепаковка уже пройдены.
 */
export function stageSegments(stage: JobStage, percent: number): StageSegment[] {
  const visited = new Set(stage.visited);
  const currentIndex = ORDER.findIndex((step) => step.id === stage.id);
  return ORDER.map((step, index) => {
    const [start, end] = BANDS[step.id];
    let state: SegmentState;
    if (step.id === stage.id) state = 'current';
    else if (visited.has(step.id)) state = 'done';
    else if (index < currentIndex) state = step.optional && !visited.has(step.id) ? 'skipped' : 'done';
    else state = 'pending';
    const fill = state === 'current' ? Math.min(1, Math.max(0, (percent - start) / (end - start))) : state === 'done' ? 1 : 0;
    return { id: step.id, label: step.label, weight: end - start, state, fill };
  });
}

/** Высота плоскости среза в сцене как доля высоты модели (1 — модель целиком). */
export function sceneClip(stageId: string, percent: number): number {
  if (stageId !== 'probe_slice' && stageId !== 'final_slice') return 1;
  const [start, end] = BANDS[stageId];
  return Math.min(1, Math.max(0, (percent - start) / (end - start)));
}
