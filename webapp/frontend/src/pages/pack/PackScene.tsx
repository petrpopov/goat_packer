import { lazy, Suspense, useMemo } from 'react';
import type { ReactNode } from 'react';
import type { PrinterProfile } from '@/api/types';
import ErrorBoundary from '@/components/ErrorBoundary';
import Icon from '@/components/Icon';
import { sceneLayout } from '@/scene/layout';
import type { ModelMesh } from '@/scene/model3d';
import { sceneClip } from '@/utils/progress';
import type { PackJob } from './usePackJob';

// three.js тяжёлый: сцена грузится отдельным чанком, только когда известен профиль.
// Одна повторная попытка: сетевой сбой при загрузке чанка иначе сразу уводит в заглушку.
const BedScene = lazy(() => import('@/scene/BedScene').catch(() => import('@/scene/BedScene')));

const HINT_CONTROLS = 'колесо — масштаб, перетаскивание — поворот, правая кнопка — сдвиг';

interface PackSceneProps {
  /** Принтер сцены; null — стола ещё нет (заглушка). */
  profile: PrinterProfile | null;
  model: ModelMesh | null;
  /** Файл слишком большой для предпросмотра. */
  modelTooLarge: boolean;
  pack: PackJob;
  /** Текст поверх сцены со спиннером, пока её нечем заполнить; null — нет. */
  busyText: string | null;
  /** Беда со сценой (файл задачи недоступен) — поверх сцены с иконкой; null — нет. */
  failedText: string | null;
  /** Панель поверх сцены («Инфо»). */
  overlay?: ReactNode;
}

/** Стол принтера с моделью и раскладкой задачи, подписи и оверлеи поверх него. */
export default function PackScene({
  profile,
  model,
  modelTooLarge,
  pack,
  busyText,
  failedText,
  overlay,
}: PackSceneProps) {
  const { job, running, check, checkRunning } = pack;
  // Ссылка на layout стабильна между опросами (structural sharing react-query) — сцена не перестраивается зря.
  const jobLayout = job?.layout;
  const placed = useMemo(() => sceneLayout(jobLayout), [jobLayout]);
  // Проверка нарезкой идёт внутри готовой задачи: стол на это время — в режиме нарезки (срез слоя).
  const mode = running || checkRunning ? 'running' : job?.status === 'SUCCEEDED' ? 'done' : 'idle';
  /** Идёт перебор раскладок, позиций ещё нет: на столе мелькают проекции модели. */
  const searching = running && job?.stage?.id === 'packing' && placed.placements.length === 0;
  const clip =
    running && job?.stage
      ? sceneClip(job.stage.id, job.progressPercent)
      : checkRunning && check?.stage
        ? sceneClip(check.stage.id, check.progressPercent)
        : 1;
  // Слишком большой файл — без предпросмотра (без тоста: упаковать его всё равно можно).
  const caption = modelTooLarge
    ? 'Модель слишком большая для предпросмотра'
    : searching
      ? 'поиск раскладки'
      : model
        ? HINT_CONTROLS
        : '';

  return (
    <div className="scene-wrap">
      {profile ? (
        <ErrorBoundary
          fallback={
            <div className="scene scene-fallback">
              Не удалось показать стол: браузер без WebGL или устаревшая версия страницы — обновите страницу
            </div>
          }
        >
          <Suspense fallback={<div className="scene" />}>
            <BedScene
              profile={profile}
              model={model}
              mode={mode}
              clip={clip}
              searching={searching}
              caption={caption}
              placements={placed.placements}
              footprintLoops={placed.footprintLoops}
              tower={placed.tower}
              usableOverride={placed.bed}
              readout={model ? `Модель: ${model.size.map((v) => Math.round(v)).join(' × ')} мм` : ''}
              objectCount={pack.resultCount}
            />
          </Suspense>
        </ErrorBoundary>
      ) : (
        <div className="scene scene-empty">{busyText ? null : 'Здесь появится стол выбранного принтера и модель'}</div>
      )}
      {busyText || failedText ? (
        <div className="scene-busy" aria-live="polite">
          <span>
            {busyText ? <span className="spinner" aria-hidden="true" /> : <Icon name="alert" />}
            {busyText ?? failedText}
          </span>
        </div>
      ) : null}
      {overlay}
    </div>
  );
}
