export interface HttpErrorRecord {
  status: number;
  method: string;
  path: string;
}

export interface ConsoleErrorRecord {
  text: string;
  locationUrl?: string;
}

const OPTIONAL_CATALOG_ICON = /^\/api\/apps\/catalog\/[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\/icon$/;
const RESOURCE_NOT_FOUND = /^Failed to load resource: the server responded with a status of 404(?: \(Not Found\))?$/;

/** Catalog icons are optional and AppIcon intentionally renders a fallback. */
export function isOptionalCatalogIcon404(error: HttpErrorRecord): boolean {
  return error.status === 404 && error.method === 'GET' && OPTIONAL_CATALOG_ICON.test(error.path);
}

/** Ignore only Chromium's resource error for an icon whose matching 404 was observed. */
export function isOptionalCatalogIconConsoleError(
  error: ConsoleErrorRecord,
  origin: string,
  optionalIcon404Urls: ReadonlySet<string>,
): boolean {
  if (!error.locationUrl || !RESOURCE_NOT_FOUND.test(error.text)) return false;
  try {
    const url = new URL(error.locationUrl);
    return url.origin === origin && optionalIcon404Urls.has(url.href);
  } catch {
    return false;
  }
}
