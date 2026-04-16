export function formatClaudeStreamLine(line: string): string | null {
  if (!line.trim()) return null

  let event: Record<string, unknown>
  try {
    event = JSON.parse(line)
  } catch {
    return null
  }

  if (event.type === 'assistant' && event.subtype === 'tool_use') {
    const tool = event as Record<string, unknown>
    const name = tool.tool_name as string
    const input = tool.tool_input as Record<string, unknown>

    switch (name) {
      case 'Read':
        return `  ▸ Read ${input.file_path}`
      case 'Write':
        return `  ▸ Write ${input.file_path}`
      case 'Edit':
        return `  ▸ Edit ${input.file_path}`
      case 'Grep':
        return `  ▸ Grep "${input.pattern}" in ${input.path ?? '.'}`
      case 'Glob':
        return `  ▸ Glob ${input.pattern}`
      case 'Bash':
        return `  ▸ Bash: ${(input.command as string).slice(0, 120)}`
      default:
        return `  ▸ ${name}`
    }
  }

  if (event.type === 'assistant' && event.subtype === 'text') {
    return event.text as string
  }

  return null
}
