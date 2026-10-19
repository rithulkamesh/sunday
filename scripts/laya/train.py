"""Fine-tunes Laya (multilingual) on the routing data. Usage: python train.py [work dir] [epochs]
Reads <work>/train.jsonl and <work>/eval_proto.jsonl (from gen_routing.py), writes the checkpoint to <work>/model.
Option order is shuffled during training: the base model is very sensitive to it."""
import json
import pathlib
import sys
import time

from huggingface_hub import snapshot_download
from laya.train import TrainConfig, finetune

work = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "laya-work")
epochs = int(sys.argv[2]) if len(sys.argv) > 2 else 5
base = snapshot_download("convaiinnovations/laya-multilingual", local_dir=str(work / "base"))
cfg = TrainConfig(epochs=epochs, micro_batch=8, grad_accum=4, shuffle_options=("choice",), max_len=384, head_max_len=256,
                  calib_max=250, eval_data=str(work / "eval_proto.jsonl"), seed=42, gradient_checkpointing=False, amp=False, log_every=20)
t0 = time.time()
summary = finetune(data=str(work / "train.jsonl"), model_dir=base, output_dir=str(work / "model"), config=cfg, device="mps")
print("done in %.0fs" % (time.time() - t0))
print(json.dumps({k: v for k, v in summary.items() if k != "history"}, indent=1, default=str)[:2000])
