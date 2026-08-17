from dataclasses import dataclass
from typing import Any


@dataclass
class Message:
    role: str
    content: str


SYSTEM_PROMPT = """You are MAVIS, a persistent desktop AI companion. You are always present but never intrusive.

CRITICAL RULES — violations break the user experience:
1. Be concise. One sentence preferred, two sentences maximum.
2. Respond ONLY to the user's immediate message.
3. NEVER generate example conversations, roleplay, or meta-commentary.
4. NEVER repeat the user's message back to them.
5. NEVER ask "How can I help you?" or similar generic follow-ups.
6. Use context from Working Memory but do not mention it explicitly.
7. If you don't know something, say so briefly.
8. NEVER use markdown, bullet points, numbered lists, or separator lines.
9. NEVER write more than one paragraph. No line breaks in your response.
10. NEVER repeat yourself or restate the same fact in multiple ways.
11. Speak like a natural human, not a robot. Use contractions and casual tone."""


def build_chat_messages(
    user_messages: list[dict[str, str]],
    working_memory: list[dict[str, Any]] | None = None,
    system_prompt: str = SYSTEM_PROMPT,
) -> list[dict[str, str]]:
    """
    Build the message list for the LLM.

    Working memory is merged INTO the system prompt so models like Phi-3
    receive a single coherent system context block.
    """
    # Merge working memory into the system prompt
    system = system_prompt
    if working_memory:
        context_lines: list[str] = []
        for item in working_memory:
            content = item.get("content", "")
            source = item.get("source", "memory")
            if content:
                context_lines.append(f"- [{source}] {content}")

        if context_lines:
            system += "\n\nWorking Memory:\n" + "\n".join(context_lines)

    messages: list[dict[str, str]] = [{"role": "system", "content": system}]

    for msg in user_messages:
        role = msg.get("role", "user")
        content = msg.get("content", "")
        # Defensive: skip any system messages from callers — we already built
        # the canonical system prompt above with working memory merged in.
        if role == "system":
            continue
        if content:
            messages.append({"role": role, "content": content})

    return messages
