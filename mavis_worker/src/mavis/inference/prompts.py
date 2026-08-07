"""Prompt templates and context injection."""

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any


@dataclass
class Message:
    role: str
    content: str


SYSTEM_PROMPT = """You are MAVIS, a persistent desktop-native AI companion. You are helpful, concise, and proactive. You run entirely locally on the user's machine. You have access to the desktop environment and can perform actions like opening apps, running shell commands, sending notifications, and controlling media.

Rules:
- Keep responses brief and actionable unless asked for detail.
- When the user asks you to do something, respond with a plan in JSON format when appropriate.
- Be friendly but professional. You are a companion, not a servant.
- If you don't know something, say so. Do not hallucinate facts.
- Current date: {current_date}"""


def build_chat_messages(
    user_message: str,
    history: list[Message] | None = None,
    context_items: list[dict[str, Any]] | None = None,
) -> list[dict[str, str]]:
    """Build an OpenAI-style message list for llama-cpp chat completion."""
    history = history or []
    context_items = context_items or []

    system_content = SYSTEM_PROMPT.format(current_date=datetime.now(tz=timezone.utc).isoformat())

    if context_items:
        ctx_lines = "\n".join(
            f"- {item.get('key', 'unknown')}: {item.get('value', '')}" for item in context_items
        )
        system_content += f"\n\nRelevant context:\n{ctx_lines}"

    messages: list[dict[str, str]] = [{"role": "system", "content": system_content}]

    for msg in history:
        messages.append({"role": msg.role, "content": msg.content})

    messages.append({"role": "user", "content": user_message})
    return messages
