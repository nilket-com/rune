"""Exact five-repeat counter rules for fresh measurements."""
import math
import statistics
from fractions import Fraction

def require(ok,reason):
    if not ok:raise ValueError(reason)
def assess(num,den):
    require(type(num) is int and type(den) is int and num>0 and den>0,'positive exact counter medians required')
    ratio=Fraction(num,den)
    return dict(ratio=float(ratio),exact_numerator=ratio.numerator,exact_denominator=ratio.denominator,
                rejected=ratio>Fraction(201,200))
def assess_cycles(candidate,tip):
    require(len(candidate)==len(tip)==5,'five cycle repeats required')
    candidate=[count(value) for value in candidate];tip=[count(value) for value in tip]
    candidate_median=statistics.median(candidate);tip_median=statistics.median(tip)
    ratio=Fraction(candidate_median,tip_median)
    tip_spread=Fraction(max(tip)-min(tip),tip_median)
    candidate_spread=Fraction(max(candidate)-min(candidate),candidate_median)
    threshold=1+max(2*tip_spread,Fraction(1,50))
    return dict(candidate_median=candidate_median,tip_median=tip_median,
        ratio=float(ratio),percent_change=float((ratio-1)*100),
        tip_spread=float(tip_spread),candidate_spread=float(candidate_spread),
        threshold=float(threshold),flagged=ratio>threshold,
        exact_numerator=ratio.numerator,exact_denominator=ratio.denominator,
        threshold_numerator=threshold.numerator,threshold_denominator=threshold.denominator)

def count(value):
    require(type(value) in (int,float) and math.isfinite(value) and 0<value<=2**53 and int(value)==value,'counter is not an exact positive integer')
    return int(value)
