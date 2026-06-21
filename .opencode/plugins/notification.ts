import type { Plugin } from "@opencode-ai/plugin";

export const NotificationPlugin: Plugin = async ({ client, $ }) => {
  return {
    event: async ({ event }) => {
      // Helper to get session title
      const getSessionTitle = async (id) => {
        try {
          const session = await client.session.get({ path: { id } });
          return session?.data.title || "Unknown Session";
        } catch (e) {
          return "Session";
        }
      };

      // finish the session
      if (event.type === "session.idle") {
        const title = await getSessionTitle(event.properties.sessionID);
        await $`osascript -e 'display notification "Finished: ${title}" with title "OpenCode"'`;
      }

      // session is pending when asked question
      if (
        event.type === "permission.asked" ||
        event.type === "question.asked"
      ) {
        const title = await getSessionTitle(event.properties.sessionID);
        await $`osascript -e 'display notification "Action required in: ${title}" with title "OpenCode"'`;
      }
    },
  };
};
