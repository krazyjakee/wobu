# Narrative workspace mockup prompt

Generated with the built-in image generation tool. Visual concept for the planned narrative system.

Use case: ui-mockup
Asset type: High-fidelity desktop application UI concept, a single complete screen for the planned Narrative mode of Wobu.
Primary request: Create a polished, plausible, carefully typeset visual mockup of the Wobu narrative authoring workspace described below. This is a practical desktop content authoring application. The image should look like an implementable professional application screenshot, with clean aligned UI, correct spelling, readable controls and consistent spacing.

Composition: One straight-on, edge-to-edge desktop app screen in landscape, approximately 2048 x 1280 (16:10). No monitor, laptop, desk, browser chrome, perspective, outer poster border or decorative captions. Four vertical regions below a shallow app titlebar: narrow icon rail 3%, scene navigator 16%, main working editor 58%, right context inspector 23%. Slim full-width bottom status bar. Main pane is generously readable. Use compact professional Inter-style sans-serif, approximately 15–17px body at this canvas size, 28px scene title. Avoid illegibly small text. Plenty of breathing room within the dense functional layout. Crisp flat UI, subtle 1px separators, lightly rounded 6–10px controls. No glassmorphism, no large gradients, no oversized cards or dashboard charts.

Use Wobu's actual existing dark-theme palette: page #0d0e12, panels #14161c, raised/input surfaces #1a1d25 / #0f1117, subtle borders #2b303c, primary text #e7e9f0, secondary text #9aa1b3. Warm amber #e2a44f marks selected navigation and author actions. Violet #9d7cf5 is reserved for AI generation actions and Generated badges. Green #5fce8b indicates approved/saved, and restrained amber indicates Edited or Out of date. Render a lowercase 'wobu' wordmark as text, no invented elaborate logo. Minimal fine-line icons.

TOP BAR: lowercase 'wobu' at left, project dropdown 'Ashfall', subtle divider then 'Narrative'. Centre a quiet search box 'Find scene or line…' with keyboard hint. Right show 'Review' with a small 8 badge, 'Build' with chevron, and outlined 'Export'. Keep hierarchy clear, not all primary buttons.

LEFT ICON RAIL: five simple navigation icons for library, forge, assets, narrative and settings. The narrative icon is selected with amber left indicator and muted amber tile. Settings anchored near bottom.

NAVIGATOR: header 'Narrative' and small plus button. Search 'Filter…'. Section rows 'Scenes' with 12 count, 'Quests' with 4 count, 'World state', 'Text library'. Under Scenes show 'Beacon aftermath', expanded selected 'Council hearing', and 'The captain’s quarters'. Council hearing contains three indented beats: '01  Present evidence' selected with amber left edge, '02  Challenge the story', '03  The verdict'. A small muted '6 lines · 3 choices' under the selected beat. A 'WORK QUEUE' label lower down with rows 'Needs text  12', 'Needs review  8', 'Out of date  3', each with an icon as well as its discreet number. Bottom quiet '+ New scene'.

MAIN EDITOR:
Breadcrumb 'Scenes / Council hearing'.
Title 'Council hearing' with small neutral 'Scene' badge. One-line subtitle 'Win the council’s support without exposing Mira.'
Below title, three small circular avatar initials K, M, O with names 'Kael', 'Mira', 'Orren'. Use initials, no fantasy portraits or decorative paintings.
Tab row 'Outline', 'Dialogue', 'Preview', 'Source', with 'Dialogue' clearly active using an amber underline.
Under tabs a thin scenario bar: 'Variant' then dropdown 'High trust · Witnessed', and a right-aligned outlined play-icon 'Preview scene' button.
Beat header '01  Present evidence' with subheading 'Kael accuses the captain. Mira supports him. Orren stays sceptical.'
A compact collapsible strip 'Scene condition' followed by a subdued monospaced chip 'beacon_quest = investigating'.

DIALOGUE editor arranged as three elegant stacked line rows with subtle backgrounds, not chat bubbles:
First row: small K initials avatar, 'KAEL', metadata line 'Generated' in violet, 'Draft' neutral, 'Current' subdued. Text in larger readable type: 'The beacon didn’t fail. Someone gave the order to shut it down.' Fine hover-like pencil and more icons on right, no send buttons.
Second row: small M avatar, 'MIRA', 'Edited' amber, 'Approved' green, 'Current' subdued. Text: 'I saw the signal log. Kael is telling the truth.' A tiny lower-right 'View changes' action, not a giant call-to-action.
Third row: small O avatar, 'ORREN', lock icon and 'Locked' neutral, 'Approved' green, 'Out of date' amber. Text: 'An accusation is not evidence. What can you actually prove?' A compact amber informational line underneath: 'Context changed · Review required'. This line demonstrates that a locked text can still be out of date. Do not show regenerate controls on this locked row.

Below dialogue, separated by whitespace and a rule, a 'Player choices' heading with a modest '+ Add choice' action.
Three clean table columns labeled 'Choice', 'Requires', 'Consequence'.
Row 1: 'Show the logbook' / 'Has logbook' / 'Council support +10'.
Row 2: 'Appeal to their duty' / 'Always' / 'Continue to challenge'.
Row 3: 'Threaten the council' / 'Always' / 'Orren trust −10'.
Use small destination chevrons and restrained green/amber for positive/negative numeric effects. These are authored deterministic outcomes, not AI suggestions.

At bottom of main pane a quiet '+ Add line' control and a violet primary action 'Generate missing text' with small sparkle icon. Accompany with muted 'Locked lines are protected'. Ensure the important content is fully visible with no arbitrary cutoff.

RIGHT INSPECTOR:
Header 'Context' with small info icon.
A section 'SELECTED VARIANT' with two compact pills 'High trust' and 'Witnessed'.
'WHO KNOWS WHAT' section, three neatly divided rows:
'Kael' / 'Witnessed the beacon attack' / small provenance chip 'First-hand'
'Mira' / 'Knows the signal log was altered' / chip 'Evidence'
'Orren' / 'Heard rumours about the captain' / chip 'Rumour'
Link 'View world facts' below.
'RELATIONSHIPS' section showing 'Mira → Kael' with 'Trust 71' and a slender muted green meter. 'Orren → Kael' with small amber chip 'Distrustful'.
'SCENE INTENT' section text 'Challenge the captain’s story. Keep the alliance intact.'
'DO NOT REVEAL' section with subtle amber shield/lock icon and text 'Mira changed the signal log.' Subtext 'Reserved for a later scene.'
'DEPENDENCIES' section with concise '3 characters · 2 facts · 1 quest' and outlined text action 'Inspect generation request'.
No giant inspector call-to-action and no irrelevant image generation settings.

BOTTOM STATUS: small green dot 'All changes saved' at left; middle '1 line needs review'; right 'Narrative workspace · Concept'.
Constraints: Accurate clear text with correct names Kael, Mira, Orren. Clear separation between authored logic and generated prose. Distinct generation policy, approval, freshness labels. Visual polish and practical desktop proportions. No runtime chatbot, no chat input box, no graphs with tangled wires, no marketing hero, no invented AI assistant panel, no giant artwork, no stock photos, no watermark. The image is a UI design proposal, not evidence of an already shipped feature.
