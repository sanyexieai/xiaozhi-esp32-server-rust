import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GO_DIR = ROOT.parent / "xiaozhi-esp32-server-golang" / "manager" / "backend" / "controllers"
VOICE_CONSTANTS = GO_DIR / "voice_constants.go"
VOICES_GO = GO_DIR / "voices.go"
DATA_DIR = ROOT / "crates" / "xiaozhi-manager" / "data"


def parse_voice_constants(path: Path) -> dict[str, list[dict[str, str]]]:
    text = path.read_text(encoding="utf-8")
    providers: dict[str, list[dict[str, str]]] = {}
    current = None
    for line in text.splitlines():
        m = re.match(r'\s*"([a-z0-9_]+)":\s*\{', line)
        if m:
            current = m.group(1)
            providers[current] = []
            continue
        m = re.match(r'\s*\{Value:\s*"([^"]*)",\s*Label:\s*"([^"]*?)"\}', line)
        if m and current:
            providers[current].append({"value": m.group(1), "label": m.group(2)})
    return providers


def parse_qwen_model_voices(path: Path) -> dict[str, list[dict[str, str]]]:
    text = path.read_text(encoding="utf-8")
    models: dict[str, list[dict[str, str]]] = {}
    current = None
    for line in text.splitlines():
        m = re.match(r'\s*"([^"]+)":\s*\{', line)
        if m and "ModelVoiceMap" in text[: text.find(line)]:
            current = m.group(1)
            models[current] = []
            continue
        m = re.match(
            r'\s*\{Value:\s*"([^"]*)",\s*Label:\s*"([^"]*?)"(?:,\s*Description:.*)?\}',
            line,
        )
        if m and current:
            models[current].append({"value": m.group(1), "label": m.group(2)})
    return models


def main() -> None:
    providers = parse_voice_constants(VOICE_CONSTANTS)
    providers["openai"] = [
        {"value": "alloy", "label": "Alloy"},
        {"value": "echo", "label": "Echo"},
        {"value": "fable", "label": "Fable"},
        {"value": "onyx", "label": "Onyx"},
        {"value": "nova", "label": "Nova"},
        {"value": "shimmer", "label": "Shimmer"},
    ]
    qwen_models = parse_qwen_model_voices(VOICES_GO)

    DATA_DIR.mkdir(parents=True, exist_ok=True)
    (DATA_DIR / "voice_options.json").write_text(
        json.dumps(providers, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    (DATA_DIR / "qwen_voices_by_model.json").write_text(
        json.dumps(qwen_models, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    for key, voices in providers.items():
        print(f"{key}: {len(voices)}")
    for key, voices in qwen_models.items():
        print(f"qwen/{key}: {len(voices)}")


if __name__ == "__main__":
    main()
