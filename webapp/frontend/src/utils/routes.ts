/** Маршруты SPA. nginx и Vite отдают index.html на любой путь, поэтому это настоящие URL. */
export type LoginStep = 'signin' | 'register' | 'forgot' | 'sent' | 'code';

export type Route =
  | { name: 'pack' }
  | { name: 'schedule' }
  | { name: 'calibration' }
  | { name: 'jobs' }
  | { name: 'job'; id: string }
  | { name: 'admin' }
  | { name: 'login'; step: LoginStep };

const LOGIN_STEPS: LoginStep[] = ['register', 'forgot', 'sent', 'code'];
const JOB_ID = /^[a-z0-9-]+$/;

export function parseRoute(pathname: string): Route {
  const [head, tail] = pathname.split('/').filter(Boolean);
  switch (head) {
    case 'schedule':
      return { name: 'schedule' };
    case 'calibration':
      return { name: 'calibration' };
    case 'admin':
      return { name: 'admin' };
    case 'jobs':
      return tail && JOB_ID.test(tail) ? { name: 'job', id: tail } : { name: 'jobs' };
    case 'login':
      return { name: 'login', step: LOGIN_STEPS.find((step) => step === tail) ?? 'signin' };
    default:
      return { name: 'pack' };
  }
}

export function routePath(route: Route): string {
  switch (route.name) {
    case 'job':
      return `/jobs/${route.id}`;
    case 'login':
      return route.step === 'signin' ? '/login' : `/login/${route.step}`;
    default:
      return `/${route.name}`;
  }
}
