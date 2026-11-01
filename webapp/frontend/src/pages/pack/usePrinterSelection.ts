import { useEffect, useMemo } from 'react';
import type { PrinterProfile } from '@/api/types';
import { CUSTOM_PRINTER } from '@/components/PrinterPicker';
import { useProfileDetection } from '@/hooks/useProfileDetection';
import { errorMessage } from '@/utils/jobs';
import {
  customBedFromProject,
  customPrinterProfile,
  isStlName,
  parseCustomBed,
  type CustomBedInput,
} from '@/utils/printers';

/**
 * Принтер задачи: определён по 3MF (выбор заблокирован), выбран пользователем (STL или до файла),
 * свой стол («Моего принтера нет в списке» или 3MF неизвестного принтера — размеры из проекта).
 */
export type PrinterChoice =
  { kind: 'detected'; id: string } | { kind: 'picked'; id: string } | { kind: 'custom' } | null;

/**
 * Состояние файла: idle (нет) / detecting (загрузка и определение) / ok (можно паковать) / unknown
 * (3MF без printer_model — его не принять даже со своим столом) / error.
 */
export type DetectionState = 'idle' | 'detecting' | 'ok' | 'unknown' | 'error';

const NO_PRINTER_MODEL = 'В проекте не указан printer_model. Сохраните проект с выбранным принтером в слайсере.';

interface SelectionInput {
  file: File | null;
  /** Выбор в PrinterPicker (id или CUSTOM_PRINTER); определение по 3MF его перекрывает. */
  picked: string | null;
  profiles: PrinterProfile[];
  bed: CustomBedInput;
  setBed: (bed: CustomBedInput) => void;
}

/** Определение принтера по файлу черновика и выбор пользователя → профиль упаковки. */
export function usePrinterSelection({ file, picked, profiles, bed, setBed }: SelectionInput) {
  const detectQuery = useProfileDetection(file);
  const detected = detectQuery.data;
  const stl = file !== null && isStlName(file.name);
  const detectedId = !stl ? (detected?.profileId ?? null) : null;
  // 3MF неизвестного принтера: пакер примет его только со своим столом (сверяет printer_model с профилем).
  const unknown3mf = file !== null && !stl && detectQuery.isSuccess && detectedId === null;
  const projectBed = unknown3mf ? (detected?.projectBed ?? null) : null;

  const choice: PrinterChoice = detectedId
    ? { kind: 'detected', id: detectedId }
    : unknown3mf
      ? { kind: 'custom' }
      : file !== null && !stl
        ? null // 3MF ещё определяется или не прочитался
        : picked === CUSTOM_PRINTER
          ? { kind: 'custom' }
          : picked
            ? { kind: 'picked', id: picked }
            : null;
  const custom = choice?.kind === 'custom';
  const profileId = choice && choice.kind !== 'custom' ? choice.id : '';

  // Стол из проекта неизвестного принтера — в поля своего стола: размеры только для показа (слайсер
  // режет проект по своему столу, сервер правку отвергнет), отступ — свой.
  useEffect(() => {
    if (projectBed) setBed(customBedFromProject(projectBed));
  }, [projectBed, setBed]);

  const customBed = parseCustomBed(bed);
  const customKey = customBed ? `${customBed.width}|${customBed.depth}|${customBed.height}|${customBed.margin}` : '';
  // Сцена перестраивает стол при смене объекта профиля: синтетический профиль — только при смене размеров.
  const customProfile = useMemo(
    () => (customBed ? customPrinterProfile(customBed, projectBed) : null),
    // customBed пересоздаётся на каждом рендере; его значения — в customKey.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [customKey, projectBed]
  );
  const selectedProfile = custom ? customProfile : (profiles.find((profile) => profile.id === profileId) ?? null);

  const detection: DetectionState =
    file === null
      ? 'idle'
      : detectQuery.isPending
        ? 'detecting'
        : detectQuery.isError
          ? 'error'
          : unknown3mf && !detected?.printerModel
            ? 'unknown'
            : 'ok';
  const detectionError = detectQuery.isError
    ? errorMessage(detectQuery.error, 'Не удалось проверить файл')
    : detection === 'unknown'
      ? NO_PRINTER_MODEL
      : null;
  const detecting3mf = file !== null && !stl && detection === 'detecting';

  const pickerHint = detecting3mf
    ? 'Определяю принтер по файлу…'
    : unknown3mf
      ? projectBed
        ? 'Принтер не в каталоге — размеры взяты из проекта'
        : `Принтер${detected?.printerModel ? ` «${detected.printerModel}»` : ''} не в каталоге — укажите размеры стола`
      : stl
        ? 'STL: выберите принтер — проект соберём на сервере'
        : file === null
          ? 'можно выбрать заранее — стол появится справа'
          : undefined;

  return {
    upload: detectQuery.upload,
    choice,
    custom,
    profileId,
    unknown3mf,
    projectBed,
    customBed,
    selectedProfile,
    detection,
    detectionError,
    detecting3mf,
    pickerHint,
  };
}

export type PrinterSelection = ReturnType<typeof usePrinterSelection>;
