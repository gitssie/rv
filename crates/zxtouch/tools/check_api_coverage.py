"""Compare SDK coverage inventory with an adjacent current TrollVNC checkout."""
import argparse
import re
import sys
from pathlib import Path


def rust_routes():
    source = (Path(__file__).resolve().parents[1]/"src/lib.rs").read_text()
    paths = set()
    for prefix, block in re.findall(r'namespace!\(\s*\w+,\s*"([^"]+)",\s*\[(.*?)\]\s*\);', source, re.S):
        paths.update(prefix + wire for wire in re.findall(r'=>\s*"([^"]+)"', block))
    # Operation wrappers use literal invoke methods, shared result/stream and raw routes.
    for ty, prefix in (("Apple", "apple"), ("AppState", "app-state")):
        block = source.split("impl<'a> " + ty + "<'a> {", 1)[1].split("\n}", 1)[0]
        paths.update("/api/" + prefix + "/" + method for method in re.findall(r'\.invoke\("([^"]+)"', block))
    paths.update("/api/" + prefix + "/result" for prefix in ("apple", "app-state"))
    paths.update("/api/apple/" + method for method in ("submit_code", "cancel_challenge"))
    paths.update(("/api/app-state/stream", "/api/karing/*"))
    paths.update(re.findall(r'"(/api/(?:device/(?:screenshot|lock|unlock)|app-state/(?:upload|download)))"', source))
    # connect is delegated to the transport authentication implementation.
    transport = (Path(__file__).resolve().parents[1]/"src/transport.rs").read_text()
    if '"/api/control/connect"' in transport:
        paths.add("/api/control/connect")
    return paths



def server_routes(server):
    source = (server / "src/TVNCZXTouchService.mm").read_text()
    paths = {p for p in re.findall(r'@"(/api/[^" ]+)"', source) if not p.endswith("/")}
    apple = (server / "src/TVNCAppleService.mm").read_text().split("static NSDictionary *Methods(void)",1)[1].split("@implementation",1)[0]
    paths.update("/api/apple/" + name for name in re.findall(r'@"(\w+)"\s*:\s*@\[', apple))
    paths.update("/api/apple/" + name for name in ("result", "submit_code", "cancel_challenge"))
    paths.add("/api/app-state/result")
    service = (server / "vendor/app-state/src/TVAppStateServices.m").read_text()
    found = re.search(r'if\(!\[@\[(.*?)\]containsObject:payload\[@"operation"\]\]\)', service)
    if not found:
        raise RuntimeError("Active AppState admission list changed; inspect source before updating coverage")
    names = re.findall(r'@"(\w+)"', found.group(1))
    protocol = (server / "vendor/app-state/src/TVASProtocol.m").read_text()
    mapping = dict(re.findall(r'@"(\w+)":@"(\w+)"', protocol))
    paths.update("/api/app-state/" + wire for wire, native in mapping.items() if native in names)
    native = source.split('hasPrefix:@"/api/karing-native/"', 1)[1].split("BOOL selecting", 1)[0]
    allowed = native.split("if (![@[",1)[1].split("] containsObject",1)[0]
    paths.update("/api/karing-native/" + name for name in re.findall(r'@"(\w+)"', allowed))
    paths.add("/api/karing/*")
    return paths


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, default=Path(__file__).resolve().parents[4]/"TrollVNC")
    args = parser.parse_args()
    expected = server_routes(args.server)
    actual = rust_routes()
    missing = expected - actual; stale = actual - expected
    if missing or stale:
        print("Missing SDK routes:", sorted(missing)); print("Stale SDK routes:", sorted(stale))
        return 1
    print(f"Covered {len(expected)-1} named routes and the complete /api/karing/* proxy")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
