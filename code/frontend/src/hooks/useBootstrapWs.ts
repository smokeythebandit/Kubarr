import { useState, useEffect, useRef, useCallback } from 'react';
import { useQuery } from '@tanstack/react-query';
import { setupApi, ComponentStatus } from '../api/setup';

export type BootstrapConnectionMode = 'websocket' | 'polling' | 'disconnected';

interface BootstrapEvent {
  type: 'initial_status' | 'component_started' | 'component_progress' | 'component_completed' | 'component_failed' | 'bootstrap_complete';
  component?: string;
  message?: string;
  error?: string;
  progress?: number;
  components?: ComponentStatus[];
  complete?: boolean;
}

interface UseBootstrapWsResult {
  components: ComponentStatus[];
  isComplete: boolean;
  isStarted: boolean;
  isConnected: boolean;
  connectionMode: BootstrapConnectionMode;
  error: Error | null;
}

const MAX_RECONNECT_DELAY = 30000; // 30 seconds
const INITIAL_RECONNECT_DELAY = 1000; // 1 second
const POLLING_INTERVAL = 2000; // 2 seconds

export function useBootstrapWs(): UseBootstrapWsResult {
  const [components, setComponents] = useState<ComponentStatus[]>([]);
  const [isComplete, setIsComplete] = useState(false);
  const [isStarted, setIsStarted] = useState(false);
  const [connectionMode, setConnectionMode] = useState<BootstrapConnectionMode>('disconnected');
  const [error, setError] = useState<Error | null>(null);

  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reconnectDelayRef = useRef(INITIAL_RECONNECT_DELAY);
  const mountedRef = useRef(true);

  // Fallback to HTTP polling when WebSocket is not connected
  const shouldPoll = connectionMode !== 'websocket';

  const { data: polledStatus } = useQuery({
    queryKey: ['bootstrap', 'status', 'polling'],
    queryFn: setupApi.getBootstrapStatus,
    refetchInterval: shouldPoll ? POLLING_INTERVAL : false,
    enabled: shouldPoll,
  });

  // Use polled data as fallback when not using WebSocket
  useEffect(() => {
    if (shouldPoll && polledStatus) {
      setComponents(polledStatus.components);
      setIsComplete(polledStatus.complete);
      setIsStarted(polledStatus.started);
      setConnectionMode('polling');
    }
  }, [shouldPoll, polledStatus]);

  // Update a single component's status based on an event
  const updateComponentStatus = useCallback((event: BootstrapEvent) => {
    if (!event.component) return;

    setComponents((prev) => {
      return prev.map((comp) => {
        if (comp.component !== event.component) return comp;

        switch (event.type) {
          case 'component_started':
            return {
              ...comp,
              status: 'installing' as const,
              message: event.message || 'Installing...',
              error: undefined,
            };
          case 'component_progress':
            return {
              ...comp,
              status: 'installing' as const,
              message: event.message || comp.message,
            };
          case 'component_completed':
            return {
              ...comp,
              status: 'healthy' as const,
              message: event.message || 'Installed successfully',
              error: undefined,
            };
          case 'component_failed':
            return {
              ...comp,
              status: 'failed' as const,
              message: event.message || 'Installation failed',
              error: event.error,
            };
          default:
            return comp;
        }
      });
    });

    // Mark as started if we get any component event
    setIsStarted(true);
  }, []);

  const connect = useCallback(() => {
    if (!mountedRef.current) return;
    if (wsRef.current?.readyState === WebSocket.OPEN) return;

    // Build WebSocket URL from current location
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${protocol}//${window.location.host}/api/setup/bootstrap/ws`;

    try {
      const ws = new WebSocket(wsUrl);
      wsRef.current = ws;

      ws.onopen = () => {
        if (!mountedRef.current) return;
        console.log('Bootstrap WebSocket connected');
        setConnectionMode('websocket');
        setError(null);
        reconnectDelayRef.current = INITIAL_RECONNECT_DELAY;
      };

      ws.onmessage = (event) => {
        if (!mountedRef.current) return;
        try {
          const message: BootstrapEvent = JSON.parse(event.data);

          switch (message.type) {
            case 'initial_status':
              if (message.components) {
                setComponents(message.components);
              }
              if (message.complete !== undefined) {
                setIsComplete(message.complete);
              }
              break;
            case 'component_started':
            case 'component_progress':
            case 'component_completed':
            case 'component_failed':
              updateComponentStatus(message);
              break;
            case 'bootstrap_complete':
              setIsComplete(true);
              break;
          }
        } catch (e) {
          console.error('Failed to parse WebSocket message:', e);
        }
      };

      ws.onerror = (event) => {
        console.error('Bootstrap WebSocket error:', event);
        setError(new Error('WebSocket connection error'));
      };

      ws.onclose = (event) => {
        if (!mountedRef.current) return;
        console.log('Bootstrap WebSocket closed:', event.code, event.reason);
        wsRef.current = null;

        // Switch to polling immediately
        setConnectionMode('polling');

        // Schedule reconnection with exponential backoff
        if (mountedRef.current) {
          const delay = reconnectDelayRef.current;
          console.log(`Scheduling WebSocket reconnection in ${delay}ms`);

          reconnectTimeoutRef.current = setTimeout(() => {
            if (mountedRef.current) {
              connect();
            }
          }, delay);

          // Exponential backoff (max 30 seconds)
          reconnectDelayRef.current = Math.min(delay * 2, MAX_RECONNECT_DELAY);
        }
      };
    } catch (e) {
      console.error('Failed to create WebSocket:', e);
      setError(e instanceof Error ? e : new Error('Failed to create WebSocket'));
      setConnectionMode('polling');

      // Schedule reconnection
      reconnectTimeoutRef.current = setTimeout(() => {
        if (mountedRef.current) {
          connect();
        }
      }, reconnectDelayRef.current);

      reconnectDelayRef.current = Math.min(reconnectDelayRef.current * 2, MAX_RECONNECT_DELAY);
    }
  }, [updateComponentStatus]);

  useEffect(() => {
    mountedRef.current = true;
    connect();

    return () => {
      mountedRef.current = false;

      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
        reconnectTimeoutRef.current = null;
      }

      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
    };
  }, [connect]);

  return {
    components,
    isComplete,
    isStarted,
    isConnected: connectionMode !== 'disconnected',
    connectionMode,
    error,
  };
}
