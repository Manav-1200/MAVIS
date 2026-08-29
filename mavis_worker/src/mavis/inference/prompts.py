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
    # Separate user_profile items from general working memory
    profile_lines: list[str] = []
    context_lines: list[str] = []

    if working_memory:
        for item in working_memory:
            content = item.get("content", "")
            source = item.get("source", "memory")
            if not content:
                continue
            if source == "user_profile":
                # Inject profile facts directly into the system prompt body
                profile_lines.append(content)
            else:
                context_lines.append(f"- [{source}] {content}")

    # Echo deduplication: if a [user] line is contained in or very similar to
    # the most recent [mavis] line, it's STT picking up TTS output — drop it.
    filtered_context: list[str] = []
    last_mavis: str | None = None
    for line in context_lines:
        if line.startswith("- [mavis]"):
            last_mavis = line[len("- [mavis] ") :].strip().lower()
            filtered_context.append(line)
        elif line.startswith("- [user]") and last_mavis is not None:
            user_text = line[len("- [user] ") :].strip().lower()
            # Drop if the user "message" is substantially the same as MAVIS's last output
            if user_text in last_mavis or last_mavis in user_text:
                continue
            filtered_context.append(line)
        else:
            filtered_context.append(line)

    # Assemble system prompt: base + profile facts + working memory bullets
    system = system_prompt
    if profile_lines:
        system += "\n\n" + "\n".join(profile_lines)
    if filtered_context:
        system += "\n\nWorking Memory:\n" + "\n".join(filtered_context)

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
