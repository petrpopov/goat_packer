import { useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { serverApi } from '@/api/http';
import Icon from '@/components/Icon';
import { useLinkHandler } from '@/hooks/useLinkHandler';
import { pushToast } from '@/store/useToastStore';
import { useUiStore } from '@/store/useUiStore';
import { authRequest, PASSWORD_MAX, PASSWORD_MIN } from '@/utils/auth';
import type { LoginStep } from '@/utils/routes';

function PasswordField(props: {
  label: string;
  autoComplete: 'current-password' | 'new-password';
  value: string;
  onChange: (value: string) => void;
  aside?: ReactNode;
}) {
  const [shown, setShown] = useState(false);
  const isNew = props.autoComplete === 'new-password';
  return (
    <div className="field">
      <span className="field-label label-row">
        <label htmlFor="pw">{props.label}</label>
        {props.aside}
      </span>
      <div className="input-suffix">
        <input
          id="pw"
          className="input"
          type={shown ? 'text' : 'password'}
          autoComplete={props.autoComplete}
          value={props.value}
          onChange={(e) => props.onChange(e.target.value)}
          required
          minLength={isNew ? PASSWORD_MIN : undefined}
          maxLength={PASSWORD_MAX}
        />
        <button type="button" className="icon-btn" onClick={() => setShown((v) => !v)} aria-label={shown ? 'Скрыть пароль' : 'Показать пароль'}>
          <Icon name={shown ? 'eye-off' : 'eye'} />
        </button>
      </div>
      {isNew && <span className="field-hint">минимум {PASSWORD_MIN} символов</span>}
    </div>
  );
}

/**
 * Вход по паттерну web.dev: одна карточка, «Забыли пароль?» у поля пароля, показ пароля вместо
 * повтора, разные подписи кнопок («Войти» / «Создать аккаунт»), введённое не стирается при ошибке.
 * Email и письма — заготовки: бэкенд их пока не поддерживает (сброс — кодом от администратора).
 */
export default function LoginPage({ step }: { step: LoginStep }) {
  const [login, setLogin] = useState('');
  const [password, setPassword] = useState('');
  const [email, setEmail] = useState('');
  const [token, setToken] = useState('');
  const [error, setError] = useState<string | null>(null);
  const navigate = useUiStore((s) => s.navigate);
  const onLink = useLinkHandler();
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: async () => {
      const request = authRequest(step, { login, password, token, email });
      if (request) await serverApi.post(request.path, request.body);
    },
    onSuccess: async () => {
      setPassword('');
      // /auth/reset только меняет пароль и сессию не выдаёт (Identity.kt): после кода — снова вход.
      if (step === 'code') {
        pushToast('Пароль изменён, войдите', 'success');
        navigate('/login');
        return;
      }
      await queryClient.invalidateQueries({ queryKey: ['session'] });
      navigate('/pack');
    },
    onError: (e) => setError(e instanceof Error ? e.message : 'Не удалось выполнить запрос'),
  });

  const submit = (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    if (step === 'forgot') navigate('/login/sent');
    else mutation.mutate();
  };
  const link = (href: string, text: string) => (
    <a className="link" href={href} onClick={onLink}>
      {text}
    </a>
  );
  const back = <p className="auth-foot">{link('/login', 'Вернуться ко входу')}</p>;
  const loginField = (label: string) => (
    <div className="field">
      <label className="field-label" htmlFor="lg">{label}</label>
      <input id="lg" className="input" autoComplete="username" value={login} onChange={(e) => setLogin(e.target.value)} required minLength={3} maxLength={120} />
    </div>
  );
  const submitButton = (text: string) => (
    <button className="btn btn-primary btn-lg btn-block" disabled={mutation.isPending}>
      {text}
    </button>
  );
  const errorBlock = error ? <div className="error-block" role="alert">{error}</div> : null;

  let body: ReactNode;
  if (step === 'register') {
    body = (
      <>
        <h1>Создать аккаунт</h1>
        <p className="auth-sub">Задачи будут храниться без ограничения 24 часа</p>
        <form onSubmit={submit}>
          {loginField('Логин')}
          <div className="field">
            <label className="field-label" htmlFor="em">
              Email <span className="field-hint">необязательно</span>
            </label>
            <input id="em" className="input" type="email" autoComplete="email" value={email} onChange={(e) => setEmail(e.target.value)} />
            <span className="field-hint">
              для восстановления пароля <span className="stub-badge">скоро</span>
            </span>
          </div>
          <PasswordField label="Пароль" autoComplete="new-password" value={password} onChange={setPassword} />
          {errorBlock}
          {submitButton('Создать аккаунт')}
        </form>
        <div className="auth-note">Ваши гостевые задачи перейдут в новый аккаунт.</div>
        <p className="auth-foot">Уже есть аккаунт? {link('/login', 'Войти')}</p>
      </>
    );
  } else if (step === 'forgot') {
    body = (
      <>
        <h1>Восстановление пароля</h1>
        <p className="auth-sub">Пришлём ссылку для нового пароля на email аккаунта</p>
        <form onSubmit={submit}>
          {loginField('Логин или email')}
          {submitButton('Отправить ссылку')}
        </form>
        <div className="auth-note">Есть одноразовый код от администратора? {link('/login/code', 'Ввести код')}</div>
        {back}
      </>
    );
  } else if (step === 'sent') {
    body = (
      <>
        <h1>Проверьте почту</h1>
        <p className="auth-sub">Если у аккаунта указан email, ссылка уже в пути. Она действует 1 час.</p>
        <div className="auth-note">
          <span className="stub-badge">заготовка</span> Письма пока не отправляются. Сейчас пароль сбрасывает администратор — {link('/login/code', 'ввести его код')}.
        </div>
        {back}
      </>
    );
  } else if (step === 'code') {
    body = (
      <>
        <h1>Новый пароль</h1>
        <p className="auth-sub">Код выдаёт администратор, он одноразовый</p>
        <form onSubmit={submit}>
          <div className="field">
            <label className="field-label" htmlFor="tk">Код</label>
            <input id="tk" className="input mono" autoComplete="one-time-code" value={token} onChange={(e) => setToken(e.target.value)} required />
          </div>
          <PasswordField label="Новый пароль" autoComplete="new-password" value={password} onChange={setPassword} />
          {errorBlock}
          {submitButton('Сохранить и войти')}
        </form>
        {back}
      </>
    );
  } else {
    body = (
      <>
        <h1>Вход</h1>
        <p className="auth-sub">Goat Packer — упаковка 3D-объектов для печати</p>
        <form onSubmit={submit}>
          {loginField('Логин')}
          <PasswordField
            label="Пароль"
            autoComplete="current-password"
            value={password}
            onChange={setPassword}
            aside={
              <a className="link" style={{ fontSize: 12 }} href="/login/forgot" onClick={onLink}>
                Забыли пароль?
              </a>
            }
          />
          {errorBlock}
          {submitButton('Войти')}
        </form>
        <p className="auth-foot">Нет аккаунта? {link('/login/register', 'Создать аккаунт')}</p>
        <div className="auth-note">Можно работать и без входа: гостевые задачи хранятся 24 часа. {link('/pack', 'К упаковке')}</div>
      </>
    );
  }
  return (
    <div className="auth">
      <div className="auth-card">{body}</div>
    </div>
  );
}
