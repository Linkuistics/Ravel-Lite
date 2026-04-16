import { type LLMPhase } from '../../types.js'
import { formatToolCall, type FormattedOutput } from '../../format.js'

interface ContentBlock {
  type: string
  text?: string
  name?: string
  input?: Record<string, unknown>
}

interface AssistantMessage {
  content: ContentBlock[]
}

interface StreamEvent {
  type: string
  subtype?: string
  message?: AssistantMessage
  result?: string
}

export function formatClaudeStreamLine(line: string, phase?: LLMPhase): FormattedOutput | null {
  if (!line.trim()) return null

  let event: StreamEvent
  try {
    event = JSON.parse(line)
  } catch {
    return null
  }

  // Assistant messages contain content blocks (tool_use, text, thinking)
  if (event.type === 'assistant' && event.message?.content) {
    const outputs: FormattedOutput[] = []

    for (const block of event.message.content) {
      if (block.type === 'tool_use' && block.name && block.input) {
        const input = block.input
        switch (block.name) {
          case 'Read':
            outputs.push(formatToolCall({ name: block.name, path: input.file_path as string }, phase))
            break
          case 'Write':
            outputs.push(formatToolCall({ name: block.name, path: input.file_path as string }, phase))
            break
          case 'Edit':
            outputs.push(formatToolCall({ name: block.name, path: input.file_path as string }, phase))
            break
          case 'Grep':
            outputs.push(formatToolCall({ name: block.name, detail: `"${input.pattern}" in ${input.path ?? '.'}` }, phase))
            break
          case 'Glob':
            outputs.push(formatToolCall({ name: block.name, detail: input.pattern as string }, phase))
            break
          case 'Bash':
            outputs.push(formatToolCall({ name: block.name, detail: (input.command as string).slice(0, 120) }, phase))
            break
          default:
            outputs.push(formatToolCall({ name: block.name }, phase))
        }
      }

      if (block.type === 'text' && block.text) {
        outputs.push({ text: block.text, persist: true })
      }
    }

    // Return the last output (most relevant for single-line display)
    if (outputs.length > 0) return outputs[outputs.length - 1]
  }

  // Final result
  if (event.type === 'result' && event.result) {
    return { text: event.result, persist: true }
  }

  return null
}
