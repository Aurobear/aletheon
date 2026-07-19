#!/usr/bin/env python3
"""Run one coding task through the real aletheon exec entry point."""
from __future__ import annotations
import argparse, hashlib, json, os, pathlib, shutil, signal, subprocess, tempfile, time, tomllib

ROOT = pathlib.Path(__file__).resolve().parents[3]
MAX_CAPTURE = 64 * 1024

def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()

def run_bounded(argv, cwd, env, timeout):
    started = time.monotonic()
    proc = subprocess.Popen(argv, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            start_new_session=True)
    timed_out = False
    try:
        out, err = proc.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        os.killpg(proc.pid, signal.SIGTERM)
        try: out, err = proc.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL); out, err = proc.communicate()
    return {"argv": list(argv), "exit_code": proc.returncode, "timed_out": timed_out,
            "elapsed_ms": int((time.monotonic()-started)*1000),
            "stdout": out[:MAX_CAPTURE].decode(errors="replace"),
            "stderr": err[:MAX_CAPTURE].decode(errors="replace"),
            "stdout_digest": digest(out), "stderr_digest": digest(err),
            "stdout_truncated": len(out)>MAX_CAPTURE, "stderr_truncated": len(err)>MAX_CAPTURE}

def git(cwd, *args):
    return subprocess.check_output(["git", *args], cwd=cwd, stderr=subprocess.DEVNULL)

def canonical_hash(receipt):
    payload = {k:v for k,v in receipt.items() if k != "integrity_sha256"}
    return digest(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode())

def main():
    parser=argparse.ArgumentParser(); parser.add_argument("task"); parser.add_argument("--receipt")
    args=parser.parse_args(); task_path=pathlib.Path(args.task).resolve()
    task=tomllib.loads(task_path.read_text()); fixture=ROOT/"tests/coding/fixtures"/task["fixture"]
    binary=pathlib.Path(os.environ.get("ALETHEON_BIN", ROOT/"target/debug/aletheon")).resolve()
    if not binary.is_file(): raise SystemExit(f"missing real aletheon binary: {binary}; build it with scripts/cargo-agent.sh")
    with tempfile.TemporaryDirectory(prefix=f"aletheon-coding-{task['id']}-") as td:
        root=pathlib.Path(td); workspace=root/"workspace"; shutil.copytree(fixture, workspace)
        subprocess.run(["git","init","-q"],cwd=workspace,check=True)
        subprocess.run(["git","config","user.email","fixture@aletheon.invalid"],cwd=workspace,check=True)
        subprocess.run(["git","config","user.name","Aletheon Fixture"],cwd=workspace,check=True)
        subprocess.run(["git","add","."],cwd=workspace,check=True); subprocess.run(["git","commit","-qm","fixture"],cwd=workspace,check=True)
        before=digest(git(workspace,"ls-tree","-r","HEAD"))
        protected={p:digest((workspace/p).read_bytes()) for p in task["forbidden_paths"] if (workspace/p).is_file()}
        home=root/"home"; runtime_dir=root/"run"; config=root/"config"; home.mkdir(); runtime_dir.mkdir(); config.mkdir()
        env=os.environ.copy(); env.update({"HOME":str(home),"XDG_RUNTIME_DIR":str(runtime_dir),"XDG_CONFIG_HOME":str(config)})
        command=[str(binary),"--cd",str(workspace),"exec","--prompt",task["prompt"],"--output","json"]
        execution=run_bounded(command, workspace, env, int(task["timeout_secs"]))
        try: executive=json.loads(execution["stdout"])
        except json.JSONDecodeError: executive={"success":False,"operation_id":"","response":execution["stdout"]}
        operation_id=str(executive.get("operation_id", ""))
        status=git(workspace,"status","--porcelain").decode().splitlines()
        changed=sorted(line[3:] for line in status if len(line)>=4)
        diff=git(workspace,"diff","--binary","HEAD").decode(errors="replace")
        after=digest(git(workspace,"hash-object",*sorted(p for p in changed if (workspace/p).is_file()))) if changed else before
        forbidden_ok=all((workspace/p).is_file() and digest((workspace/p).read_bytes())==value for p,value in protected.items())
        scope_ok=bool(changed) and set(changed).isdisjoint(set(task["forbidden_paths"]))
        hidden = ROOT/"tests/coding/acceptance"/task["id"]
        if hidden.is_dir():
            for source in hidden.rglob("*"):
                if source.is_file():
                    destination=workspace/source.relative_to(hidden); destination.parent.mkdir(parents=True,exist_ok=True); shutil.copy2(source,destination)
        acceptance=[]
        remaining=max(1,int(task["timeout_secs"])-execution["elapsed_ms"]//1000)
        for raw in task["acceptance_commands"]:
            argv=list(raw)
            if argv and argv[0]=="cargo": argv=["bash",str(ROOT/"scripts/cargo-agent.sh"),*argv[1:]]
            result=run_bounded(argv,workspace,env,remaining); acceptance.append(result)
            if result["timed_out"]: break
        verified=bool(operation_id and executive.get("success") and not execution["timed_out"] and forbidden_ok and scope_ok and acceptance and all(x["exit_code"]==0 and not x["timed_out"] for x in acceptance))
        now=int(time.time()*1000)
        evidence=[{"operation_id":operation_id,"tool_call_id":f"acceptance-{i}","kind":"acceptance_command",
                   "command":" ".join(item["argv"]),"exit_code":item["exit_code"],"stdout_digest":item["stdout_digest"],
                   "stderr_digest":item["stderr_digest"],"workspace_before":before,"workspace_after":after,"observed_at_ms":now}
                  for i,item in enumerate(acceptance)]
        receipt={"schema_version":1,"task_id":task["id"],"operation_id":operation_id,
                 "events":[{"kind":"executive_completed","elapsed_ms":execution["elapsed_ms"],"exit_code":execution["exit_code"],"stdout_digest":execution["stdout_digest"],"stderr_digest":execution["stderr_digest"],"stderr":execution["stderr"],"response":str(executive.get("response", ""))[:MAX_CAPTURE]}],
                 "workspace_diff":diff,"changed_files":changed,"acceptance":acceptance,
                 "usage":{"iterations":executive.get("iterations",0),"tool_calls":executive.get("tool_calls_made",0),"elapsed_ms":executive.get("elapsed_ms",execution["elapsed_ms"])},
                 "evidence":evidence,"verification":{"passed":verified,"forbidden_paths_unchanged":forbidden_ok,"allowed_scope":scope_ok},
                 "terminal_status":"verified" if verified else "failed_verification"}
        receipt["integrity_sha256"]=canonical_hash(receipt)
        output=pathlib.Path(args.receipt) if args.receipt else ROOT/"tests/coding/receipts"/f"{task['id']}.json"
        output.parent.mkdir(parents=True,exist_ok=True); output.write_text(json.dumps(receipt,indent=2,sort_keys=True)+"\n")
        print(output)
        raise SystemExit(0 if verified else 1)
if __name__=="__main__": main()
