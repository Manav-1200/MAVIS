from dataclasses import dataclass
from typing import Any


@dataclass
class Message:
    role: str
    content: str


SYSTEM_PROMPT = """You are MAVIS, a persistent desktop AI companion. You are always present but never intrusive.

CRITICAL RULES — violations break the user experience:
1. Be concise. One or two sentences maximum.
2. Respond ONLY to the user's immediate message.
3. NEVER generate example conversations, roleplay, or meta-commentary.
4. NEVER repeat the user's message back to them.
5. NEVER ask "How can I help you?" or similar generic follow-ups.
6. Use Working Memory for context but do not mention it explicitly.
7. If you don't know something, say so briefly."""


def build_chat_messages(
    user_messages: list[dict[str, str]],
    working_memory: list[dict[str, Any]] | None = None,
    system_prompt: str = SYSTEM_PROMPT,
) -> list[dict[str, str]]:
    messages = [{"role": "system", "content": system_prompt}]

    if working_memory:
        context_lines = []
        for item in working_memory:
            content = item.get("content", "")
            source = item.get("source", "memory")
            if content:
                context_lines.append(f"- [{source}] {content}")

        if context_lines:
            context_block = "Working Memory:\n" + "\n".join(context_lines)
            messages.append({"role": "system", "content": context_block})

    for msg in user_messages:
        role = msg.get("role", "user")
        content = msg.get("content", "")
        if content:
            messages.append({"role": role, "content": content})

    return messages
