Original prompt: Create a skateboarding simulator game (state_sonnet) with a stick figure riding a skateboard. Arrow keys to lean, space bar hold to crouch/charge jump, space release to pop/jump. Simple skatepark with flat ground and boxes. Camera follows skater.

## Status: Initial implementation complete

### What's implemented
- Physics: gravity, friction, lean-based acceleration/braking, crouch-and-pop jump
- Controls: Arrow Left/Right to lean, Space hold to crouch, Space release to jump
- Push mechanic: releasing space while leaning forward adds horizontal push bonus
- Stick figure with: helmet, body, arms, legs with knee-bend on crouch, skateboard with trucks + wheels
- 4 boxes at varying distances/heights for obstacles
- Scrolling camera with look-ahead smoothing
- HUD: distance counter, best distance, speed bar, jump charge bar
- Background: night city skyline, stars, scrolling ground markings
- Start screen with controls listed
- window.advanceTime(ms) for deterministic Playwright testing
- window.render_game_to_text() for state inspection

### TODOs for next agent
- Test with Playwright and fix any physics/visual issues found
- Consider: grinds on the box rails (currently visual only)
- Consider: trick system (grab, kickflip via key combos)
- Consider: sounds (browser Web Audio API)
- Consider: more varied terrain (slopes, ramps)
