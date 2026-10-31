import { describe, expect, test } from 'bun:test';
import { authRequest, PASSWORD_MIN } from '../src/utils/auth';

describe('authRequest', () => {
  test('sign in and register send login + password to the existing endpoints', () => {
    expect(authRequest('signin', { login: ' petr ', password: 'x'.repeat(12) })).toEqual({
      path: '/auth/login',
      body: { login: 'petr', password: 'x'.repeat(12) },
    });
    expect(authRequest('register', { login: 'petr', password: 'y'.repeat(12), email: 'a@b.c' })).toEqual({
      path: '/auth/register',
      body: { login: 'petr', password: 'y'.repeat(12) },
    });
  });
  test('admin code resets the password', () => {
    expect(authRequest('code', { token: ' t0k ', password: 'z'.repeat(12) })).toEqual({
      path: '/auth/reset',
      body: { token: 't0k', password: 'z'.repeat(12) },
    });
  });
  test('email reset is a front-end stub: no request', () => {
    expect(authRequest('forgot', { login: 'petr' })).toBeNull();
    expect(authRequest('sent', {})).toBeNull();
  });
  test('password rule is 12+ characters', () => {
    expect(PASSWORD_MIN).toBe(12);
  });
});
