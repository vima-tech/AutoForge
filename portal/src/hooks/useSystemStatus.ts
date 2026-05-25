import { useQuery } from "@tanstack/react-query";
import axios from "axios";

interface ConcurrencyStatus {
  stage: "normal" | "throttled" | "paused";
  active_slots: number;
  pending_review_count: number;
  max_slots: number;
  pause_threshold: number;
  effective_concurrency: number;
}

interface SystemStatus {
  status: string;
  concurrency: ConcurrencyStatus;
}

export function useSystemStatus() {
  return useQuery<SystemStatus>({
    queryKey: ["system-status"],
    queryFn: async () => {
      const { data } = await axios.get("/api/v1/system/status");
      return data;
    },
    refetchInterval: 10_000,
  });
}
