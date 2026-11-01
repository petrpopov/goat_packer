/** Интервалы опроса, размеры страниц и пределы — единое место для чисел, о которых договорились с бэкендом. */

/** Статус открытой задачи, пока идёт она сама или её проверка нарезкой. */
export const JOB_POLL_MS = 1000;
/** Секундомер этапа между опросами. */
export const CLOCK_TICK_MS = 1000;
/** «Мои задачи» и состояние системы в админке. */
export const LIST_POLL_MS = 5000;
/** Загруженные исходники в «Моих задачах». */
export const UPLOADS_POLL_MS = 10000;
/** Сессия сервера считается свежей минуту. */
export const SESSION_STALE_MS = 60000;
/** Сколько висит уведомление. */
export const TOAST_TTL_MS = 5000;

/** Размеры страниц, которые отдаёт бэкенд (/jobs и /admin/users). */
export const JOBS_PAGE_SIZE = 20;
export const ADMIN_USERS_PAGE_SIZE = 30;
