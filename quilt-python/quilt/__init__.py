from ._quilt import *

def reduce(term):
    return eval(term.coparse())
