import { describe, it, expect, beforeEach } from 'vitest';
import {
  saveAccount,
  removeAccount,
  getOtherAccounts,
  getSavedAccounts,
  getCurrentAccountId,
} from '../accounts';

describe('accounts utilities', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('saveAccount persists account and sets current account id', () => {
    saveAccount({ userId: 1, username: 'alice', email: 'alice@test.com' });

    const accounts = getSavedAccounts();
    expect(accounts).toHaveLength(1);
    expect(accounts[0].username).toBe('alice');
    expect(accounts[0].lastSignedIn).toBeDefined();

    expect(getCurrentAccountId()).toBe(1);
  });

  it('removeAccount clears current account key when removing the active account', () => {
    saveAccount({ userId: 1, username: 'alice' });
    saveAccount({ userId: 2, username: 'bob' });

    // Current account is now 2 (last saved)
    expect(getCurrentAccountId()).toBe(2);

    removeAccount(2);

    expect(getSavedAccounts()).toHaveLength(1);
    expect(getCurrentAccountId()).toBeNull();
  });

  it('getOtherAccounts filters out the current user', () => {
    saveAccount({ userId: 1, username: 'alice' });
    saveAccount({ userId: 2, username: 'bob' });
    saveAccount({ userId: 3, username: 'charlie' });

    const others = getOtherAccounts(2);

    expect(others).toHaveLength(2);
    expect(others.map((a) => a.username)).toEqual(['alice', 'charlie']);
  });
});
