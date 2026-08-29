import { useMutation } from "@tanstack/react-query";
import type { Project } from "@/ipc";
import { commands } from "@/ipc";
import { queryClient } from "@/queries/client";
import { qk } from "@/queries/keys";

/** `<EditableProjectName>` — optimistic, same shape as `useSetTitle`. */
export function useSetProjectName(projectId: string) {
  return useMutation({
    mutationFn: (name: string) => commands.project.setName(projectId, name),
    onMutate: async (name) => {
      const key = qk.project(projectId);
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<Project>(key);
      if (previous) {
        queryClient.setQueryData<Project>(key, { ...previous, name });
      }
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(qk.project(projectId), context.previous);
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: qk.projects() });
    },
  });
}
