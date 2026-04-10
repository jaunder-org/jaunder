export const RulesyncHooksPlugin = async ({ $ }) => {
  return {
    "tool.execute.after": async (input) => {
      const __re = new RegExp("Edit|Write");
      if (__re.test(input.tool)) {
        await $`./scripts/format`;
      }
      const __re = new RegExp("Edit|Write");
      if (__re.test(input.tool)) {
        await $`jq -r '.tool_input.file_path' | { read -r f; git add "$f"; } 2>/dev/null || true`;
      }
    },
  };
};
