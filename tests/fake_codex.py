"""Deterministic local protocol fixture. It never connects to any network."""
import json, os, pathlib, sys, time

root = pathlib.Path(os.environ['CODEX_HOME'])
fixture = json.loads((root/'fixture.json').read_text()) if (root/'fixture.json').exists() else {}
ledger_path = root/'fixture-ledger.json'
ledger = json.loads(ledger_path.read_text()) if ledger_path.exists() else {'redeemed': {}, 'requests': []}

def save():
    ledger_path.write_text(json.dumps(ledger))

def emit(v):
    print(json.dumps(v), flush=True)

for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req:
        continue
    ident, method = req['id'], req['method']
    result = {}
    if method == 'initialize':
        result = {'userAgent': 'fake-test', 'platformFamily': 'unix', 'platformOs': 'linux'}
    elif method == 'account/read':
        result = {'account': {'type': 'chatgpt', 'email': 'fixture@example.invalid', 'planType': 'pro'}}
    elif method == 'account/rateLimits/read':
        now = int(time.time())
        redeemed = set(ledger['redeemed'].values())
        cards = [{'id': cid, 'expiresAt': exp, 'grantedAt': now-100, 'resetType': 'codexRateLimits', 'status': 'available'}
                 for cid, exp in [('late', now+86400), ('soon', now+3600), ('never', None)] if cid not in redeemed]
        result = {'accountId': fixture.get('accountId', 'fixture-account-a'),
                  'rateLimits': {'limitId': 'codex', 'primary': {'usedPercent': 100 if not redeemed else 0, 'windowDurationMins': 10080, 'resetsAt': now+10000}},
                  'rateLimitResetCredits': {'availableCount': len(cards), 'credits': cards}}
        # Interleaved sparse notifications must not wipe the complete reset-card list.
        emit({'method': 'account/rateLimits/updated', 'params': {'rateLimits': result['rateLimits']}})
    elif method == 'account/rateLimitResetCredit/consume':
        params = req['params']
        ledger['requests'].append(params)
        key, card = params['idempotencyKey'], params['creditId']
        if key in ledger['redeemed']:
            result = {'outcome': 'alreadyRedeemed'}
        elif ledger['redeemed']:
            result = {'outcome': 'nothingToReset'}
        else:
            ledger['redeemed'][key] = card
            result = {'outcome': 'reset'}
            save()
            if fixture.get('drop_first_response'):
                continue
        save()
    elif method == 'account/logout':
        result = {}
    elif method == 'account/login/start':
        result = {'type': 'chatgpt', 'loginId': 'fixture-login', 'authUrl': 'https://auth.openai.com/fixture'}
    elif method == 'account/login/cancel':
        result = {'status': 'canceled'}
    else:
        emit({'id': ident, 'error': {'code': -32601, 'message': 'Method not found'}})
        continue
    emit({'id': ident, 'result': result})

