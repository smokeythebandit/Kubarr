import { useQuery } from '@tanstack/react-query'
import { FlowWindow, networkingApi } from '../api/networking'

/**
 * Flow classes come from PromQL `rate()` over a window, so they change on the
 * scrape interval rather than continuously. Refetching faster than this only
 * re-renders the same numbers.
 */
const FLOW_CLASS_REFETCH_MS = 15000

export function useFlowClasses(window: FlowWindow = '15m') {
  return useQuery({
    queryKey: ['networking', 'flow-classes', window],
    queryFn: () => networkingApi.getFlowClasses(window),
    refetchInterval: FLOW_CLASS_REFETCH_MS,
    // A wider window is a strictly larger query; keeping the previous answer on
    // screen while it runs stops the whole panel flashing empty on every switch.
    placeholderData: previous => previous,
  })
}
