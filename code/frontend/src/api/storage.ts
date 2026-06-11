import apiClient from './client';

export interface FileInfo {
  name: string;
  path: string;
  type: 'file' | 'directory';
  size: number;
  modified: string;
  permissions: string;
}

export interface DirectoryListing {
  path: string;
  parent: string | null;
  items: FileInfo[];
  total_items: number;
}

export interface StorageStats {
  total_bytes: number;
  used_bytes: number;
  free_bytes: number;
  usage_percent: number;
}

export const storageApi = {
  // Browse a directory
  browse: async (path: string = ''): Promise<DirectoryListing> => {
    const response = await apiClient.get<DirectoryListing>('/storage/browse', {
      params: { path },
    });
    return response.data;
  },

  // Get storage statistics
  getStats: async (): Promise<StorageStats> => {
    const response = await apiClient.get<StorageStats>('/storage/stats');
    return response.data;
  },

  // Get file or directory info
  getFileInfo: async (path: string): Promise<FileInfo> => {
    const response = await apiClient.get<FileInfo>('/storage/file-info', {
      params: { path },
    });
    return response.data;
  },

  // Create a new directory
  createDirectory: async (path: string): Promise<{ success: boolean; message: string }> => {
    const response = await apiClient.post<{ success: boolean; message: string }>('/storage/mkdir', {
      path,
    });
    return response.data;
  },

  // Delete a file or empty directory
  deletePath: async (path: string): Promise<{ success: boolean; message: string }> => {
    const response = await apiClient.delete<{ success: boolean; message: string }>('/storage/delete', {
      params: { path },
    });
    return response.data;
  },

  // Get download URL for a file
  getDownloadUrl: (path: string): string => {
    const baseUrl = apiClient.defaults.baseURL || '/api';
    return `${baseUrl}/storage/download?path=${encodeURIComponent(path)}`;
  },
};

// Helper function to format bytes to human readable
export function formatBytes(bytes: number, decimals: number = 2): string {
  if (bytes === 0) return '0 Bytes';

  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB', 'PB'];

  const i = Math.floor(Math.log(bytes) / Math.log(k));

  return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
}

// Helper function to format date
export function formatDate(dateString: string): string {
  const date = new Date(dateString);
  return date.toLocaleDateString() + ' ' + date.toLocaleTimeString();
}
