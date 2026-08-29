from mnemos_worker.diff_guardrail import significant_change_ratio


def test_identical_text_is_not_significant():
    ratio, significant = significant_change_ratio("same text here", "same text here")
    assert significant is False
    assert ratio == 1.0


def test_entirely_new_text_is_significant():
    ratio, significant = significant_change_ratio(
        "alpha bravo charlie delta echo foxtrot golf hotel india juliet",
        "zzzzzzzz yyyyyyyy xxxxxxxx wwwwwwww vvvvvvvv uuuuuuuu tttttttt",
    )
    assert significant is True
    assert ratio < 0.20


def test_fifty_percent_churn_is_not_significant():
    old = "a b c d e f g h i j"
    new = "a b c d e X Y Z W Q"
    ratio, significant = significant_change_ratio(old, new)
    assert significant is False
    assert ratio >= 0.20


def test_whitespace_only_churn_is_not_significant():
    old = "line one\nline two\n  line three"
    new = "line one line two line three"
    ratio, significant = significant_change_ratio(old, new)
    assert significant is False
    assert ratio == 1.0


def test_new_project_sentinel_never_significant():
    ratio, significant = significant_change_ratio(None, "Brand new overview text.")
    assert significant is False
    assert ratio == 1.0
