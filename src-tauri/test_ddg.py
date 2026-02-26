import urllib.request
import urllib.parse
import re

query = '今天的新闻'
data = urllib.parse.urlencode({'q': query, 'kl': 'wt-wt'}).encode('utf-8')
req = urllib.request.Request('https://lite.duckduckgo.com/lite/', data=data)
req.add_header('User-Agent', 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36')
try:
    response = urllib.request.urlopen(req)
    html = response.read().decode('utf-8')
    snippets = re.findall(r"<td class='result-snippet'>([\s\S]*?)</td>", html)
    print(f"Found {len(snippets)} snippets")
    for i, s in enumerate(snippets[:5]):
        s_clean = re.sub(r"<[^>]+>", " ", s).replace("&nbsp;", " ")
        print(f"{i}: {s_clean.strip()[:100]}")
except Exception as e:
    print(e)
