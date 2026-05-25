import { useEffect, useRef } from "react";
import { useQueryClient } from "@tanstack/react-query";

export function useWebSocket(projectId: string | null) {
  const ws = useRef<WebSocket | null>(null);
  const queryClient = useQueryClient();

  useEffect(() => {
    if (!projectId) return;

    const url = `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}/ws/${projectId}`;
    ws.current = new WebSocket(url);

    ws.current.onmessage = (event) => {
      const msg = JSON.parse(event.data) as { type: string; payload: unknown };
      // Invalidate relevant queries based on message type
      switch (msg.type) {
        case "review_needed":
          queryClient.invalidateQueries({ queryKey: ["issues"] });
          queryClient.invalidateQueries({ queryKey: ["change-requests"] });
          break;
        case "preview_ready":
        case "preview_terminated":
          queryClient.invalidateQueries({ queryKey: ["previews"] });
          break;
        case "worktree_update":
          queryClient.invalidateQueries({ queryKey: ["change-requests"] });
          break;
        case "system_status":
          queryClient.invalidateQueries({ queryKey: ["system-status"] });
          break;
      }
    };

    ws.current.onerror = () => {};

    return () => {
      ws.current?.close();
    };
  }, [projectId, queryClient]);
}
