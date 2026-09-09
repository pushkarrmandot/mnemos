"""The user's own identity reaching the two prompts.

The model needs to know who the user is in order to attribute action items
correctly, so the *self* contact must arrive in the prompt as a stated
instruction, not as one key inside a dict repr the model may or may not
read — that is the part these tests pin, since it's easy to regress
silently. A prompt that still renders is not a prompt that still works, so
the assertions are about the words, not the shape.
"""

from mnemos_worker.prompts import (
    EXTRACTION_SYSTEM_PROMPT,
    build_extraction_prompt,
    build_refresh_prompt,
)

SELF = {"display_name": "Priya Raman", "first_name": "Priya", "is_self": True}
OTHER = {"display_name": "Marcus Webb", "first_name": "Marcus"}

TRANSCRIPT = {
    "turns": [
        {
            "speaker_label": "Them",
            "text": "Priya, can you send the SOW?",
            "ts_start_ms": 12000,
        },
        {"speaker_label": "You", "text": "Yes, by Friday.", "ts_start_ms": 15500},
    ]
}


def test_self_contact_is_named_and_marked_as_the_user():
    prompt = build_extraction_prompt(
        TRANSCRIPT, [SELF, OTHER], None, {"duration_s": 600}
    )

    assert "Priya Raman" in prompt
    assert "THIS IS THE USER" in prompt
    # The non-self contact is listed but must not carry the marker, or every
    # participant reads as the user.
    marked = [line for line in prompt.splitlines() if "THIS IS THE USER" in line]
    assert len(marked) == 1
    assert "Priya Raman" in marked[0]
    assert "Marcus" not in marked[0]


def test_no_contacts_omits_the_section_entirely():
    prompt = build_extraction_prompt(TRANSCRIPT, [], None, {"duration_s": 600})
    assert "Known contacts" not in prompt
    # A user who skipped the name step must not produce a dangling header or
    # an empty list the model might try to interpret.
    assert "THIS IS THE USER" not in prompt


def test_contact_without_a_display_name_falls_back_rather_than_rendering_none():
    prompt = build_extraction_prompt(
        TRANSCRIPT, [{"first_name": "Priya", "is_self": True}], None, {}
    )
    assert "Priya" in prompt
    assert "None" not in prompt.split("Transcript:")[0]


def test_transcript_lines_carry_the_timestamp_prefix_the_prompt_describes():
    prompt = build_extraction_prompt(TRANSCRIPT, [], None, {})
    # `source_timestamp_ms` is explained in terms of a `[123ms]` prefix; if the
    # renderer ever stops emitting it, the instruction becomes a lie.
    assert "[12000ms] Them: Priya, can you send the SOW?" in prompt


def test_extraction_prompt_forbids_them_as_an_assignee():
    assert 'Never write "Them"' in EXTRACTION_SYSTEM_PROMPT
    # And still explains what the channel labels mean, since the transcript
    # itself is labelled that way.
    assert '"You" is' in EXTRACTION_SYSTEM_PROMPT


def test_refresh_prompt_names_the_user_and_asks_for_second_person():
    prompt = build_refresh_prompt(
        None,
        [],
        {"name": "Acme Migration", "user": SELF},
    )
    assert "Acme Migration" in prompt
    assert "Priya Raman" in prompt
    assert '"you"' in prompt


def test_refresh_prompt_without_a_user_says_nothing_about_one():
    prompt = build_refresh_prompt(None, [], {"name": "Acme Migration", "user": None})
    assert "Acme Migration" in prompt
    assert "The user whose project this is" not in prompt


def test_refresh_prompt_renders_the_project_name_not_the_whole_meta_dict():
    prompt = build_refresh_prompt(None, [], {"name": "Acme Migration", "user": SELF})
    # Regression guard: this used to be an f-string of the raw dict, which put
    # `{'name': ..., 'user': {...}}` — braces, quotes and all — in front of the
    # model as the project's identity.
    assert "{'name'" not in prompt
    assert "is_self" not in prompt
