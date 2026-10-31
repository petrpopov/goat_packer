/** Этапы упаковки для степпера и строки статуса (протокол stage-событий colored_3mf_packer). */

import type { JobStage, PackStageId } from '../api/types';

export type StepState = 'done' | 'current' | 'pending';

export interface PackStep {
  id: PackStageId;
  label: string;
  state: StepState;
}

/** Канонический порядок этапов; необязательные показываются, только если пакер в них заходил. */
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

/**
 * Шаги степпера: текущий этап подсвечен; пройденные — посещённые или стоящие раньше текущего
 * (раскладку «сеткой» пакер пропускает). Повторная нарезка после перепаковки подсвечивает
 * «Нарезку итоговой пластины», а «Перепаковка» остаётся отмеченной пройденной.
 */
export function packSteps(stage: JobStage): PackStep[] {
  const visited = new Set(stage.visited);
  const currentIndex = ORDER.findIndex((step) => step.id === stage.id);
  return ORDER.filter((step) => !step.optional || visited.has(step.id) || step.id === stage.id).map((step) => {
    const index = ORDER.indexOf(step);
    let label = step.label;
    if (step.id === stage.id && stage.objects != null && (step.id === 'final_slice' || step.id === 'window_slice')) {
      label += ` (${objectsText(stage.objects)})`;
    }
    const state: StepState =
      step.id === stage.id ? 'current' : visited.has(step.id) || index < currentIndex ? 'done' : 'pending';
    return { id: step.id, label, state };
  });
}

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
