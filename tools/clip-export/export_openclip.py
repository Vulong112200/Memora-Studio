"""Dev-time export of OpenCLIP ViT-B/32 (laion2b_s34b_b79k) for Memora Studio (docs/MODEL_PROPOSAL.md).

Runs in tools/clip-export/.venv only (torch is never shipped). Fully offline: reads the verified checkpoint.
Outputs (installed via the Model Manager "Install from files"):
  openclip_image.xml/.bin  image encoder as OpenVINO IR (input 1x3x224x224 float, CLIP-normalized)
  openclip_tags.json       fixed bilingual tag taxonomy with L2-normalized text embeddings
Then checks IR vs PyTorch CPU on real scene frames (cosine >= 0.999, assumed threshold).

Usage: python export_openclip.py <checkpoint.safetensors> <out dir> [frames dir]
"""
import json
import os
import sys
from pathlib import Path

os.environ["HF_HUB_OFFLINE"] = "1"  # never touch the network

import numpy as np
import open_clip
import openvino as ov
import torch
from PIL import Image

# ponytail: fixed taxonomy (model card advice, English-only text side); free-text search needs the text encoder + tokenizer.
TAGS = [
    ("beach", "bãi biển"), ("sea", "biển"), ("mountain", "núi"), ("forest", "rừng"), ("river", "sông"),
    ("lake", "hồ"), ("waterfall", "thác nước"), ("rice field", "ruộng lúa"), ("city street", "đường phố"),
    ("night city lights", "thành phố về đêm"), ("sunset", "hoàng hôn"), ("sunrise", "bình minh"),
    ("snow", "tuyết"), ("rain", "mưa"), ("sky and clouds", "bầu trời và mây"), ("aerial view", "góc nhìn trên cao"),
    ("road trip on a highway", "đường đi"), ("boat", "thuyền"), ("airplane", "máy bay"), ("car", "xe hơi"),
    ("motorbike", "xe máy"), ("bicycle", "xe đạp"), ("hiking trail", "đường mòn"), ("big tree", "cây cổ thụ"),
    ("camping", "cắm trại"), ("food", "đồ ăn"), ("drinks", "đồ uống"), ("restaurant", "nhà hàng"),
    ("market", "chợ"), ("temple", "đền chùa"), ("church", "nhà thờ"), ("building", "toà nhà"),
    ("indoor room", "trong nhà"), ("people", "con người"), ("group of friends", "nhóm bạn"),
    ("family", "gia đình"), ("child", "trẻ em"), ("selfie", "ảnh tự sướng"), ("dog", "chó"), ("cat", "mèo"),
    ("birthday party", "tiệc sinh nhật"), ("wedding", "đám cưới"), ("concert", "buổi hoà nhạc"),
    ("sports", "thể thao"), ("swimming pool", "hồ bơi"), ("park", "công viên"), ("flowers", "hoa"),
    ("fireworks", "pháo hoa"),
]
TEMPLATE = "a photo of {}."
# Bad-shot detectors (own prompts, flagged negative): the Editing Core penalises windows that match them.
NEGATIVE = [
    ("finger over the lens", "ngón tay che ống kính", "a photo where a finger covers part of the camera lens"),
    ("blurry shot", "ảnh nhoè", "a very blurry, out of focus photo"),
    ("camera pointing at the ground", "quay xuống đất", "an accidental photo of the ground or pavement"),
    ("too dark", "quá tối", "an almost black, very dark photo"),
]
# Camera cues (v3, owner feedback 2026-11-08): scored as a two-way softmax between a prompt and its contrast,
# separately from the tag list (mixing them into the 52-way softmax dilutes them). Checked on 850 frames of the
# owner's Nam Cát Tiên footage: "view ahead" ≥ 0.5 = camera raised, road and landscape visible; < 0.05 =
# camera tilted down at handlebars and the ground; walking / ferry / river shots score 0.6–0.9.
CUES = [
    ("view ahead", "thấy đường phía trước",
     "a first-person photo from a bicycle showing the road far ahead and the landscape",
     "a photo looking down at bicycle handlebars and the ground right in front of the wheel"),
    # v4 (owner feedback 2026-11-11: forest trails, small bridges and slopes were missing — "view ahead" scored
    # 87 % of the riding samples < 0.1 because handlebars are always in frame). Checked on 1 010 frames (one per
    # 4 s) of the Nam Cát Tiên footage: forest trail ≥ 0.9 = dirt path / forest road, paved open road 0.01–0.4;
    # small bridge ≥ 0.6 = log and plank crossings (river views also score high, so the Editing Core requires the
    # forest trail cue too); slope ≥ 0.95 = pushing bikes up forest paths, steep trail sections. "Leaf-covered
    # path" and "ground only" prompt pairs did not separate and were left out.
    ("forest trail", "đường mòn trong rừng",
     "a first-person photo riding a bicycle on a narrow dirt path through a dense green forest",
     "a first-person photo riding a bicycle on a wide paved road in the open"),
    ("small bridge", "qua cầu nhỏ",
     "a photo of a small bridge over a stream in the forest",
     "a photo of a path in the forest without any bridge"),
    ("slope", "đường dốc trong rừng",
     "a photo of a steep dirt path going up or down a hill",
     "a photo of a flat level path"),
]
MODEL = "ViT-B-32"


def main(ckpt, out, frames=None):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    torch.manual_seed(0)
    model, _, preprocess = open_clip.create_model_and_transforms(MODEL, pretrained=ckpt)
    model.eval()

    example = torch.randn(1, 3, 224, 224)
    ov_model = ov.convert_model(model.visual, example_input=example)
    ov_model.reshape({0: [1, 3, 224, 224]})
    ov.save_model(ov_model, out / "openclip_image.xml")  # FP16-compressed weights (OpenVINO default)

    tokenizer = open_clip.get_tokenizer(MODEL)
    with torch.no_grad():
        t = model.encode_text(tokenizer([TEMPLATE.format(en) for en, _ in TAGS] + [prompt for _, _, prompt in NEGATIVE]))
        t = torch.nn.functional.normalize(t, dim=-1).numpy()
        cue = torch.nn.functional.normalize(model.encode_text(tokenizer([p for c in CUES for p in c[2:]])), dim=-1).numpy()
    emb = lambda e: [round(float(x), 6) for x in e]
    (out / "openclip_tags.json").write_text(json.dumps({
        "model": f"{MODEL} laion2b_s34b_b79k", "template": TEMPLATE, "logitScale": float(model.logit_scale.exp()),
        "preprocess": {"size": 224, "resize": "shortest side, bicubic, center crop",
                       "mean": list(preprocess.transforms[-1].mean), "std": list(preprocess.transforms[-1].std)},
        "tags": [{"en": en, "vi": vi, "embedding": [round(float(x), 6) for x in e]} for (en, vi), e in zip(TAGS, t)]
        + [{"en": en, "vi": vi, "negative": True, "embedding": [round(float(x), 6) for x in e]}
           for (en, vi, _), e in zip(NEGATIVE, t[len(TAGS):])],
        "cues": [{"en": en, "vi": vi, "embedding": emb(cue[2 * i]), "contrast": emb(cue[2 * i + 1])}
                 for i, (en, vi, _, _) in enumerate(CUES)],
    }, ensure_ascii=False), encoding="utf-8")

    # Consistency: IR (as installed, FP16 weights) vs PyTorch CPU.
    imgs = sorted(Path(frames).rglob("*.jpg"))[:40] if frames else []
    batch = torch.stack([preprocess(Image.open(p).convert("RGB")) for p in imgs]) if imgs else torch.randn(8, 3, 224, 224)
    compiled = ov.Core().compile_model(out / "openclip_image.xml", "CPU")
    with torch.no_grad():
        ref = model.encode_image(batch).numpy()
    got = np.concatenate([compiled(b[None].numpy())[0] for b in batch])
    cos = (ref * got).sum(1) / (np.linalg.norm(ref, axis=1) * np.linalg.norm(got, axis=1))
    # Top tag must agree too (what the app actually uses).
    agree = float(np.mean((ref @ t.T).argmax(1) == (got @ t.T).argmax(1)))
    print(json.dumps({"images": len(batch), "realFrames": bool(imgs), "cosineMin": float(cos.min()),
                      "cosineMean": float(cos.mean()), "topTagAgreement": agree}))
    assert cos.min() >= 0.999, f"IR differs from PyTorch: min cosine {cos.min():.5f}"
    assert agree == 1.0, "top tag differs between IR and PyTorch"
    print("OK", out)


if __name__ == "__main__":
    main(*sys.argv[1:])
