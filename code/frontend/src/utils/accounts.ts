// Multi-account management utilities

export interface SavedAccount {
  userId: number;
  username: string;
  email?: string;
  lastSignedIn: string; // ISO date string
}

const ACCOUNTS_KEY = 'kubarr_saved_accounts';
const CURRENT_ACCOUNT_KEY = 'kubarr_current_account';

/**
 * Get all saved accounts from localStorage
 */
export function getSavedAccounts(): SavedAccount[] {
  try {
    const stored = localStorage.getItem(ACCOUNTS_KEY);
    if (!stored) return [];
    return JSON.parse(stored);
  } catch {
    return [];
  }
}

/**
 * Save or update an account in the saved accounts list
 */
export function saveAccount(account: Omit<SavedAccount, 'lastSignedIn'>): void {
  const accounts = getSavedAccounts();
  const existingIndex = accounts.findIndex(a => a.userId === account.userId);

  const updatedAccount: SavedAccount = {
    ...account,
    lastSignedIn: new Date().toISOString(),
  };

  if (existingIndex >= 0) {
    accounts[existingIndex] = updatedAccount;
  } else {
    accounts.push(updatedAccount);
  }

  localStorage.setItem(ACCOUNTS_KEY, JSON.stringify(accounts));
  localStorage.setItem(CURRENT_ACCOUNT_KEY, account.userId.toString());
}

/**
 * Remove an account from the saved accounts list
 */
export function removeAccount(userId: number): void {
  const accounts = getSavedAccounts();
  const filtered = accounts.filter(a => a.userId !== userId);
  localStorage.setItem(ACCOUNTS_KEY, JSON.stringify(filtered));

  // If we removed the current account, clear the current account key
  const currentId = localStorage.getItem(CURRENT_ACCOUNT_KEY);
  if (currentId && parseInt(currentId) === userId) {
    localStorage.removeItem(CURRENT_ACCOUNT_KEY);
  }
}

/**
 * Get the current account ID (the one that's actively signed in)
 */
export function getCurrentAccountId(): number | null {
  const id = localStorage.getItem(CURRENT_ACCOUNT_KEY);
  return id ? parseInt(id) : null;
}

/**
 * Set the current account ID
 */
export function setCurrentAccountId(userId: number): void {
  localStorage.setItem(CURRENT_ACCOUNT_KEY, userId.toString());
}

/**
 * Get other accounts (not the currently signed-in one)
 */
export function getOtherAccounts(currentUserId: number | null): SavedAccount[] {
  const accounts = getSavedAccounts();
  if (currentUserId === null) return accounts;
  return accounts.filter(a => a.userId !== currentUserId);
}

/**
 * Clear all saved accounts (for sign out of all accounts)
 */
export function clearAllAccounts(): void {
  localStorage.removeItem(ACCOUNTS_KEY);
  localStorage.removeItem(CURRENT_ACCOUNT_KEY);
}
