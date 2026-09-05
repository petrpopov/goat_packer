import { useRef, useState } from 'react';
import type { DragEvent } from 'react';
import { pushToast } from '@/store/useToastStore';
import { formatBytes } from '@/utils/format';

const MAX_FILE_SIZE_BYTES = 200 * 1024 * 1024;

interface FileDropProps {
  file: File | null;
  onFileChange: (file: File | null) => void;
  disabled?: boolean;
}

function validateFile(file: File): string | null {
  if (!file.name.toLowerCase().endsWith('.3mf')) return 'Поддерживаются только файлы .3mf';
  if (file.size > MAX_FILE_SIZE_BYTES) return 'Файл больше 200 МБ';
  return null;
}

export default function FileDrop({ file, onFileChange, disabled = false }: FileDropProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [dragOver, setDragOver] = useState(false);

  const accept = (candidate: File | null | undefined) => {
    if (!candidate) return;
    const problem = validateFile(candidate);
    if (problem) {
      pushToast(problem, 'error');
      if (inputRef.current) inputRef.current.value = '';
      onFileChange(null);
      return;
    }
    onFileChange(candidate);
  };

  const handleDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setDragOver(false);
    if (disabled) return;
    accept(event.dataTransfer.files?.[0]);
  };

  return (
    <div
      className={`file-drop${dragOver ? ' is-drag' : ''}${file ? ' has-file' : ''}`}
      role="button"
      tabIndex={disabled ? -1 : 0}
      aria-disabled={disabled}
      aria-label="Загрузить файл .3mf"
      onClick={() => {
        if (!disabled) inputRef.current?.click();
      }}
      onKeyDown={(event) => {
        if (!disabled && (event.key === 'Enter' || event.key === ' ')) {
          event.preventDefault();
          inputRef.current?.click();
        }
      }}
      onDragOver={(event) => {
        event.preventDefault();
        if (!disabled) setDragOver(true);
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={handleDrop}
    >
      <svg
        className="file-drop-icon"
        viewBox="0 0 24 24"
        width="22"
        height="22"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <path d="M12 16V4" />
        <path d="m7 9 5-5 5 5" />
        <path d="M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3" />
      </svg>
      {file ? (
        <>
          <span className="file-name mono">{file.name}</span>
          <span className="file-size">{formatBytes(file.size)} · нажмите, чтобы выбрать другой файл</span>
        </>
      ) : (
        <>
          <span className="file-name">Перетащите файл .3mf сюда</span>
          <span className="file-size">или нажмите для выбора · до 200 МБ</span>
        </>
      )}
      <input
        ref={inputRef}
        type="file"
        accept=".3mf"
        className="file-drop-input"
        onChange={(event) => {
          accept(event.target.files?.[0]);
        }}
      />
    </div>
  );
}
