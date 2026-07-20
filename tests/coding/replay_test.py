#!/usr/bin/env python3
import copy, hashlib, importlib.util, json, pathlib, tempfile, unittest
HERE=pathlib.Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location("replay",HERE/"harness/replay.py"); replay=importlib.util.module_from_spec(spec); spec.loader.exec_module(replay)
def seal(value):
    body={k:v for k,v in value.items() if k!="integrity_sha256"}; value["integrity_sha256"]="sha256:"+hashlib.sha256(json.dumps(body,sort_keys=True,separators=(",", ":")).encode()).hexdigest(); return value
class ReplayTest(unittest.TestCase):
    def base(self):
        return seal({"operation_id":"op","workspace_diff":"diff","evidence":[{"operation_id":"op","kind":"acceptance_command","exit_code":0}],"acceptance":[{"exit_code":0,"timed_out":False}],"verification":{"passed":True},"terminal_status":"verified"})
    def check(self,value):
        with tempfile.NamedTemporaryFile("w",delete=False) as f: json.dump(value,f); name=f.name
        return replay.verify(name)[0]
    def test_valid_and_tampering_failures(self):
        self.assertTrue(self.check(self.base()))
        tampered=self.base(); tampered["workspace_diff"]="evil"; self.assertFalse(self.check(tampered))
        for mutate in [lambda x:x.update(evidence=[]),lambda x:x["evidence"][0].update(operation_id="other"),lambda x:x["acceptance"][0].update(exit_code=1),lambda x:x["verification"].update(passed=False)]:
            value=self.base(); mutate(value); seal(value); self.assertFalse(self.check(value))
if __name__=="__main__": unittest.main()
