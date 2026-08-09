from dataclasses import dataclass
from typing import Any


@dataclass
class Message:
    role: str
    content: str


SYSTEM_PROMPT = """You are MAVIS, a persistent desktop AI companion. You are always present but never intrusive. You help the user with tasks, provide context-aware assistance, and maintain a warm, efficient personality.

Rules:
- Be concise. The user values brevity.
- If you don't know something, say so.
- Use the provided Working Memory to ground your responses.
- Do not hallucinate facts not present in the context."""


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
