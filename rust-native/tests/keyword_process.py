"""Shared legacy fixture data; final callers retire this module under #195."""
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def command(id='one',keyword='abc',target='own text',**extra):
    return dict(id=id,title=id,kind='snippet',target=target,keyword=keyword,aliases=[],arguments=[],**extra)
def catalog(*commands):return dict(op='catalog',commands=list(commands))
def typed(text):return dict(op='type',text=text)
