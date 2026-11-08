//! Training sentences for the command model, made from patterns: every
//! intent has many ways to say it, with fillers ("hey nexa", "please"),
//! names put in (apps, files, workspaces, numbers) and typos added. The
//! same seed gives the same sentences, so the model is reproducible.
//!
//! Pattern syntax: `[a|b|]` picks one (an empty choice: nothing),
//! `{APP}` `{FILE}` `{WS}` `{WHO}` `{TOPIC}` `{NUM}` `{LAYER}` put in a name or number; the
//! name's words are the argument the tagger learns to find.

use crate::rng::Rng;

pub struct Example {
    pub text: String,
    pub intent: usize,
    /// Which words are the argument (an app, file or workspace name).
    pub arg: Vec<bool>,
}

/// (intent, patterns). The order is the model's label order: never reorder
/// without retraining.
pub const INTENTS: &[(&str, &[&str])] = &[
    ("open_downloads", &["[open|show|show me|go to|take me to|browse|view] [the|my|] [downloads|download folder|downloads folder|downloaded files|downloads directory]",
                         "where are [my|the] downloads", "[files|stuff|things] i downloaded", "downloads"]),
    ("open_documents", &["[open|show|show me|go to|browse] [the|my|] [documents|documents folder|docs|document folder]", "[my|] documents", "where are my documents"]),
    ("open_pictures", &["[open|show|show me|go to|browse] [the|my|] [pictures folder|pictures directory|images folder]", "[open|show] [the|my|] pictures folder", "pictures folder"]),
    ("open_music_folder", &["[open|show|go to|browse] [the|my|] [music folder|songs folder|music directory]", "where is my music folder"]),
    ("open_videos", &["[open|show|go to|browse] [the|my|] [videos folder|movies folder|video folder]", "videos folder", "where are my videos"]),
    ("open_desktop", &["[open|show|go to] [the|my|] desktop folder", "desktop folder", "files on my desktop"]),
    ("open_home", &["[open|show|go to] [my|the] home [folder|directory|]", "home folder", "my files", "open my home"]),
    ("open_trash", &["[open|show|show me|empty|look in] [the|my|] [trash|bin|recycle bin|rubbish bin|wastebasket]",
                     "[deleted files|things i deleted|files i threw away]", "restore something from the [trash|bin]"]),
    ("launch", &["[open|launch|start|run|fire up|bring up|load|show] {APP}", "[open|launch|start] [the|] {APP} [app|application|program|]",
                 "{APP}", "i need {APP}", "switch to {APP}", "[get|give] me {APP}", "use {APP}", "{APP} [please|now|app]"]),
    ("find_file", &["where [did|have] i [put|save|saved|move|moved|leave|left] [my|the|] {FILE}", "where is [my|the|] {FILE}",
                    "[find|locate|search for|look for] [my|the|] {FILE} [file|document|]", "i lost [my|the|] {FILE}",
                    "where did [my|the|] {FILE} go", "[can't|cant|cannot] find [my|the|] {FILE}", "where's [my|the|] {FILE}"]),
    ("workspace_go", &["[open|activate|restore|switch to|start|load|go to] [the|my|] {WS} workspace", "{WS} workspace",
                       "[set up|bring back] [my|] {WS} [workspace|setup|apps]", "workspace {WS}"]),
    ("workspace_save", &["[save|remember|keep|store|make] [these|my|the|this|] [apps|windows|setup|layout] as a workspace",
                         "[new|create a|make a|save a|save this as a] workspace", "save [this|my] workspace"]),
    ("brightness_down", &["[reduce|lower|decrease|dim|turn down] [the|] [brightness|screen brightness|screen|display]",
                          "[screen|display] [too bright|is too bright|darker]", "make [the|my|] [screen|display] [darker|dimmer|less bright]",
                          "dim [the|my|] [screen|display|]", "brightness down", "less brightness", "it's too bright"]),
    ("brightness_up", &["[increase|raise|turn up|boost] [the|] [brightness|screen brightness]", "make [the|my|] [screen|display] [brighter|lighter]",
                        "brighter [screen|]", "brightness up", "more brightness", "[screen|display] [is too dark|too dim]", "i can't see the screen it's too dark"]),
    ("brightness_set", &["[set|put|change|make] [the|] brightness [to|at|] {NUM} [percent|%|]", "brightness {NUM} [percent|%|]",
                         "[screen|display] brightness [to|at] {NUM} [percent|%|]"]),
    ("volume_down", &["[turn down|lower|reduce|decrease] the [volume|sound|audio]", "[quieter|softer|less loud]", "volume down",
                      "[it's|its|this is] too loud", "make it quieter", "turn it down", "less volume"]),
    ("volume_up", &["[turn up|raise|increase|boost] the [volume|sound|audio]", "[louder|make it louder]", "volume up", "i can't hear [it|anything|]",
                    "[it's|its] too quiet", "turn it up", "more volume", "pump up the volume"]),
    ("volume_set", &["[set|put|change] [the|] volume [to|at|] {NUM} [percent|%|]", "volume {NUM} [percent|%|]", "sound [at|to] {NUM} [percent|%]"]),
    ("mute", &["mute [the|] [sound|audio|volume|speakers|computer|]", "[silence|be quiet|shut up|no sound|sound off|turn the sound off]", "mute"]),
    ("unmute", &["unmute [the|] [sound|audio|volume|]", "[sound on|turn the sound back on|turn the sound on|i want sound]", "unmute"]),
    ("lock", &["lock [the|my|] [screen|computer|pc|laptop|]", "lock [it|up]", "lock screen", "i'm stepping away lock [it|the screen]"]),
    ("suspend", &["[put|send] [the|my|] [computer|pc|laptop|machine] to sleep", "[sleep|suspend|hibernate] [the|] [computer|pc|laptop|now|]",
                  "go to sleep", "sleep mode"]),
    ("reboot", &["[restart|reboot] [the|my|] [computer|pc|laptop|system|machine|]", "[restart|reboot] now", "do a restart"]),
    ("poweroff", &["[shut down|shutdown|power off|turn off|switch off] [the|my|] [computer|pc|laptop|system|machine]",
                   "shut [it|everything] down", "[shut down|shutdown|power off]", "i'm done for today shut down"]),
    ("mode_low_resource", &["[pause|stop|hold|halt] [the|all|] background [work|tasks|jobs|scans|indexing]", "low [resource|memory|ram] mode",
                            "[free up|save] [memory|ram|resources]", "stop [scanning|indexing] for now"]),
    ("mode_battery_saver", &["[battery saver|battery saving|power saving|power saver] [mode|on|]", "save [battery|power]",
                             "make the battery last [longer|]", "[turn on|enable|use] battery saver"]),
    ("mode_performance", &["[performance|turbo|fast|full speed|max performance] mode", "[go|run] [faster|full speed]",
                           "[boost|maximum|max|full] performance", "[turn on|enable|use] performance mode", "i need more speed"]),
    ("mode_balanced", &["[balanced|normal|standard|default] mode", "[go|switch] back to [balanced|normal] [mode|]"]),
    ("mode_auto", &["[automatic|auto] mode", "[resume|restart|continue] [the|] background [work|tasks|scans]", "let nexaos decide [the mode|]"]),
    ("answer_slow", &["why is [my|the|this|] [computer|pc|laptop|system|it|everything] [so|] [slow|sluggish|laggy|lagging|freezing|hanging]",
                      "[my|the] [computer|pc|laptop] is [slow|lagging|laggy|freezing|stuck|hanging]", "what's slowing [down|] [my|the] [computer|pc|system]",
                      "everything is [slow|laggy|lagging]", "why is the fan so loud", "why is it so [hot|busy]", "[it|this] keeps freezing"]),
    ("answer_battery", &["[battery|battery health|battery status|battery life]", "how [is|healthy is|good is] [my|the] battery",
                         "how much battery [is left|do i have|left]", "is [my|the] battery [ok|okay|dying|worn out|charging]", "battery percentage"]),
    ("answer_memory", &["[what's|what is] using [the|my|all the|] [memory|ram]", "how much [memory|ram] is [used|free|left]",
                        "[memory|ram] [usage|use|full]", "why is my [memory|ram] full"]),
    ("answer_crash", &["why did [the|my|] [app|program|window|game|browser|editor] [crash|close|quit|disappear|die]",
                       "[the|my|an|] [app|program|window] [crashed|closed by itself|disappeared|vanished|just quit]",
                       "what made [the|my|] [app|program] [crash|close]", "it crashed"]),
    ("answer_network", &["why [is there|do i have] no internet", "[the|my|] [internet|wifi|wi-fi|network|connection] [is not working|isn't working|is down|is broken|stopped working|keeps dropping]",
                         "i'm offline", "[can't|cant|cannot] connect to the internet", "no internet", "why is [the|my] [wifi|internet] [down|not working|slow]"]),
    ("answer_stopped", &["why did [something|it|the download|the copy|my task] stop", "what [stopped|paused|is paused|is waiting]",
                         "why is [it|my task|the job] [paused|waiting|frozen]"]),
    ("rewind", &["[undo|revert|rewind] [the|my|that|] [settings|setting|change|changes|theme change]", "[rewind|revert] [settings|]",
                 "go back to before i changed [the settings|the theme|that]", "put my settings back"]),
    ("settings", &["[open|show|go to|] [the|] settings", "[preferences|system settings|options|control panel|configuration]", "settings"]),
    ("settings_network", &["[open|show|change|] [wifi|wi-fi|wireless|network|internet] [settings|options|]", "connect to [wifi|wi-fi|a network|the internet]",
                           "[turn on|turn off|switch on|switch off|enable|disable] [the|] [wifi|wi-fi|wireless]", "[join|change] [the|a] [wifi|network]"]),
    ("settings_bluetooth", &["[open|show|change|] bluetooth [settings|]", "[connect|pair] [my|the|a] [headphones|earbuds|speaker|mouse|keyboard|bluetooth device]",
                             "[turn on|turn off|enable|disable] bluetooth"]),
    ("settings_personalization", &["[change|set|pick|choose] [the|my|a|] [theme|wallpaper|background|colours|colors|accent colour]",
                                   "[dark mode|light mode|personalization|personalise|personalize|appearance] [settings|]", "make it look [different|nicer]"]),
    ("settings_privacy", &["[privacy|app permissions|permissions] [settings|]", "[which|what] apps can [use|see] [my|the] [camera|files|microphone]",
                           "stop [an|the|this] app [from using|using] [the|my] [camera|files|internet]"]),
    ("settings_time", &["[change|set|fix] the [time|date|clock|timezone|time zone|keyboard layout|keyboard]", "[time|date|keyboard|clock] settings",
                        "[wrong|incorrect] [time|clock|date]"]),
    ("settings_users", &["[change|reset] [my|the] password", "[users|accounts|user accounts] [settings|]", "[add|create] [a|another|a new] [user|account]"]),
    ("settings_updates", &["[check for|install|get|look for] updates", "[update|upgrade] [the|my|] [system|computer|nexaos|software|]",
                           "are there [any|] updates", "updates"]),
    ("settings_link", &["[pair|connect|link] [my|a|the] phone", "nexa link [settings|]", "send [a file|files|this] to my phone"]),
    ("settings_intelligence", &["[intelligence|microbrain|smart features|agents] [settings|]", "what does nexaos learn about me"]),
    ("settings_power", &["[power|sleep|screen off|battery] settings", "when does the screen [turn off|go to sleep|sleep]",
                         "[change|set] [when the computer sleeps|the sleep time|power settings]"]),
    ("settings_vpn", &["[vpn|wireguard] [settings|]", "[turn on|connect|start|use] [the|a|my] vpn", "[import|add] a vpn"]),
    ("settings_startup", &["[startup|start up|autostart|startup apps|services] [settings|]", "[what|which apps] start when i log in",
                           "stop [an|this|the] app [starting|from starting] [at startup|when i log in]"]),
    ("settings_magic", &["[magic layer|living background|animated background|animations|background animation] [settings|]",
                         "[change|turn off|turn on|stop] the [moving|animated|living] background"]),
    ("settings_layers", &["[layer|layers|layer moods|strata] settings", "[name|set up|change] [my|the|a] layer[s|]",
                          "[give|set] [a|the|each] layer a mood"]),
    ("settings_notifications", &["[notification|notifications] [settings|]", "[stop|turn off|hide] [the|all|] [notifications|pop-ups|popups] from {APP}",
                                 "[change|set] how [long|] notifications [stay|show]"]),
    ("settings_display", &["[display|screen|monitor|resolution|night light|scaling] [settings|]", "[change|set] [the|my] [resolution|screen size|refresh rate|scale]",
                           "[set up|connect] [a|my|the] [second screen|monitor|projector|tv]", "[turn on|turn off] night light"]),
    ("settings_sound", &["[sound|audio|speaker|microphone] settings", "[change|pick|choose] [the|my] [speaker|output|microphone|input|headset]",
                         "my microphone [isn't working|doesn't work]"]),
    ("settings_accessibility", &["[accessibility|text size|larger text|bigger text|reduce motion|high contrast] [settings|]",
                                 "make [the|] text [bigger|larger]", "[make the|] [pointer|cursor|mouse pointer] bigger", "[stop|reduce] [the|] animations"]),
    ("layer_go", &["[go|switch|move|jump|dive] to layer {LAYER}", "layer {LAYER}", "show [me|] layer {LAYER}", "[open|take me to] layer {LAYER}"]),
    ("layer_next", &["[next|deeper|the next|a deeper] layer", "[go|dive] [one|a] layer [deeper|down]", "[dive|go] deeper", "layer down"]),
    ("layer_prev", &["[previous|back|the previous|the last|upper] layer", "[go|come] [back|up] [one|a] layer", "[back|up] a layer", "layer up"]),
    ("quiet_on", &["[do not disturb|dnd|quiet mode|focus mode]", "[turn on|enable|start|switch on] [do not disturb|quiet mode|dnd|focus mode]",
                   "[stop|hide|pause|silence|mute] [the|all|] [notifications|pop-ups|popups|alerts]", "no [more|] notifications [please|for now|]", "leave me alone"]),
    ("quiet_off", &["[turn off|disable|stop|end|switch off] [do not disturb|quiet mode|dnd|focus mode]",
                    "[show|turn on|resume|enable] [the|] [notifications|pop-ups|alerts] [again|]", "i'm back show [my|the] notifications"]),
    ("windows", &["[show|show me|open|see] [all|] [my|the|] [open|] windows", "[window overview|constellation|all my windows|every window]",
                  "[what|which] [windows|apps] [are|do i have] open", "[switch|change|go] to [another|the other|a different] window",
                  "[show|open] [the|] [window overview|constellation|overview]", "where are my windows", "show me everything [that's|i have] open",
                  "[i lost|i can't find|i can't see|where did i put] [a|my|the|that] window"]),
    ("mail_find", &["[find|search for|show me|look for|get|open] [the|my|an|that|] [email|emails|mail|message|messages] [from|by|sent by] {WHO}",
                    "[find|search for|show me|look for|open] [the|my|an|that|] [email|emails|mail|message] [about|on|regarding|with] {TOPIC}",
                    "[search|check|look in] [my|the] [mail|email|emails|inbox] for {TOPIC}",
                    "[did|has] {WHO} [email|emailed|written to|replied to|mailed] me", "[any|new] [emails|mail|messages] from {WHO}",
                    "{WHO} [email|emails|mail]", "where is the email [from|by] {WHO}", "where is the email about {TOPIC}"]),
    ("mail_write", &["[write|send|compose|start|draft] [an|a|] [email|mail|e-mail] [to|for] {WHO}", "[email|mail|e-mail] {WHO}",
                     "[write|send|compose|draft|start] [an|a] [new|] [email|mail|e-mail]", "new [email|mail|e-mail]",
                     "send {WHO} [an|a] [email|mail]", "[i need to|i want to|let me] [email|write to] {WHO}"]),
    ("destroy", &["[delete|remove|erase|wipe|destroy|get rid of|trash] [my|the|all|all my|every|] [files|photos|pictures|downloads|documents|music|everything|history]",
                  "[format|wipe|erase] [the|my|a] [disk|drive|usb|stick|hard drive|computer]", "[uninstall|remove|delete] {APP}",
                  "[kill|force quit|terminate] {APP}", "delete {FILE}", "[wipe|reset|factory reset] [the|my] [computer|pc|laptop|system]",
                  "rm -rf [everything|home|/]"]),
    ("unknown", &["{OUT}", "{OUT}", "{OUT}", "[can you|could you|please|i want you to|help me|i need to|i want to|let's] {OUT}", "{OUT} [for me|please|now|today|tomorrow]",
                  "what is [the|a|] [capital of france|meaning of life|weather|time in tokyo|square root of 2|largest planet|speed of light]",
                  "tell me [a joke|a story|about yourself|something interesting|the news]", "who [is|was] [the president|einstein|the best footballer|you]",
                  "[write|draft] [a poem|a letter|an essay|a story|code] [about|for|to] [my boss|cats|love|school|summer|python]",
                  "how do i [cook rice|learn python|tie a tie|change a tyre|make friends|lose weight]", "[translate|say] [hello|thank you] in [french|twi|spanish]",
                  "[hello|hi|hey|good morning|good evening|thanks|thank you|how are you|bye|goodbye|ok|okay|yes|no|lol|hmm]",
                  "what's the weather [today|tomorrow|in accra|like]", "[play|recommend] [a song|some music|a movie|a game]", "[what|which] [day|year|month] is it",
                  "[asdf|qwerty|xyz|blah blah|test|testing 123]", "[buy|order] [food|pizza|a ticket|shoes]", "[explain|summarise|summarize] [quantum physics|this|the news|world war 2]",
                  "[i feel|i am] [sad|bored|happy|tired]", "[can you|do you] [dance|sing|think|dream]", "how [old|tall|smart] are you",
                  "[who|what] made you", "[remind me|set an alarm|set a timer] [at 7|in 10 minutes|tomorrow|for 6am|to call mum]",
                  "[wake me|wake me up|call me|pick me up|meet me|text me|ring me] [at 6|at 7|at noon|at five|in an hour|tomorrow|by 8|at half past six]",
                  "[explain|define|describe] [photosynthesis|gravity|democracy|inflation|a black hole|machine learning|the internet]",
                  "[what|why|how] [does|do|did|is|are] [the sun|plants|birds|people|computers|vaccines|elections] [work|grow|fly|vote|shine]",
                  "[give me|suggest|list] [ideas|names|recipes|tips|facts] [for|about] [a party|my business|dinner|studying|ghana]",
                  "[how much|how many] [is|are] [a dollar in cedis|people in the world|days in a year|calories in an egg]",
                  "[is it|will it] [rain|be sunny|be cold] [today|tomorrow|this weekend]", "[good night|see you|you're great|i love you|sorry]",
                  "[calculate|what is] [2 plus 2|15 times 3|100 divided by 4|the area of a circle]"]),
];

pub const PREFIXES: &[&str] = &["", "", "", "", "please ", "hey nexa ", "nexa ", "can you ", "could you ", "could you please ", "i want to ",
                                "i'd like to ", "i would like to ", "can i ", "would you ", "ok ", "hi ", "pls ", "kindly ", "help me "];
pub const SUFFIXES: &[&str] = &["", "", "", "", " please", " now", " for me", "?", " thanks", " pls", ".", " right now", " quickly"];

/// Apps the tagger learns names from: NexaOS's own and common ones, so it
/// learns where a name is, not which names exist (the desktop resolves it).
pub const APPS: &[&str] = &["files", "photos", "music", "nexa browser", "browser", "the browser", "text editor", "editor", "calculator", "calc",
                            "terminal", "foot", "store", "nexa store", "video player", "pdf viewer", "image viewer", "pulse", "nexa pulse",
                            "system monitor", "assistant", "notes", "nexa notes", "tasks", "archive", "color picker", "characters", "clipboard",
                            "screenshot", "firefox", "chrome", "gimp", "blender", "spotify", "discord", "telegram", "vs code", "vscode", "zoom",
                            "libreoffice", "writer", "steam", "vlc", "obs", "inkscape", "krita", "audacity", "thunderbird", "signal", "slack",
                            "whatsapp", "minecraft", "word", "excel", "paint", "camera", "clock", "calendar", "weather", "maps", "backup",
                            "mail", "htop", "tor browser", "installer", "nexus"];
pub const FILES: &[&str] = &["cv", "resume", "invoice", "report", "thesis", "essay", "budget", "contract", "tax return", "receipt", "passport scan",
                             "holiday photos", "wedding photos", "lecture notes", "homework", "assignment", "presentation", "slides", "spreadsheet",
                             "project plan", "meeting notes", "bank statement", "letter", "song", "video", "screenshot", "certificate", "photo of mum",
                             "birthday video", "school report", "final draft", "notes", "pdf", "zip file", "backup", "design", "logo", "poster",
                             "exam timetable", "script", "book", "recipe", "map"];
pub const FILE_TAILS: &[&str] = &["", "", "", " 2024", " 2025", " for work", " from last week", " final", " v2", " draft", " pdf", " for the bank"];
pub const WORKSPACES: &[&str] = &["coding", "android development", "study", "work", "school", "writing", "design", "music production",
                                  "gaming", "research", "web dev", "thesis", "meetings", "photo editing"];
/// People and things mail is from or about (Mail looks the name up).
pub const PEOPLE: &[&str] = &["kofi", "ama", "abena", "yaw", "kwame", "efua", "john", "sarah", "mum", "dad", "my boss", "my teacher",
                              "my lecturer", "the landlord", "my landlord", "the bank", "the school", "hr", "the hr team", "amazon", "dhl",
                              "grandma", "my sister", "my brother", "kofi mensah", "ama boateng", "the doctor", "my manager", "the council"];
pub const TOPICS: &[&str] = &["the invoice", "exam results", "the flight", "school fees", "the meeting", "my order", "the job offer", "rent",
                              "the wedding", "the contract", "the interview", "my visa", "the party", "the delivery", "the payment",
                              "lunch on friday", "the report", "my password", "the trip", "the course"];
pub const NUMBERS: &[&str] = &["10", "20", "25", "30", "40", "50", "60", "70", "75", "80", "90", "100", "five", "fifty", "half", "thirty", "eighty"];
/// Things people ask that NexaOS's commands don't do: verbs and objects
/// mixed at random ("knit a flight" too), including command verbs on
/// things that aren't the computer's ("turn off the lights"), so the model
/// learns that the object decides, not the verb.
pub const OUT_VERBS: &[&str] = &["knit", "paint", "book", "buy", "cook", "teach me", "build", "draw", "call", "text", "order", "feed", "wash",
                                 "drive", "fix", "clean", "plant", "water", "sell", "rent", "bake", "sing", "dance", "learn", "read me",
                                 "translate", "summarise", "pay", "send money to", "invite", "marry", "visit", "walk", "count",
                                 "turn off", "turn on", "open", "close", "start", "set", "change", "show me", "find me", "get me", "make",
                                 "switch on", "switch off", "lock", "play"];
pub const OUT_OBJECTS: &[&str] = &["a scarf", "my house", "a flight", "a hotel", "a taxi", "dinner", "guitar", "a website", "the lights",
                                   "the oven", "the tv", "the fan", "the heater", "the air conditioner", "the door", "the car", "the window",
                                   "the garage", "the cat", "the dog", "the plants", "the garden", "my mum", "my friend", "a pizza", "a cake",
                                   "the dishes", "the bill", "my rent", "a song", "a dance", "french", "a story", "a poem", "a restaurant",
                                   "the weather", "a doctor", "an uber", "the kettle", "the fridge", "groceries", "a gift", "the news",
                                   "the football score", "a recipe", "my homework", "the car engine", "a hairdresser", "the bus times",
                                   "a meeting with john", "the curtains", "the gate", "my bike", "a picture of a horse", "the stars"];
pub const LAYERS: &[&str] = &["1", "2", "3", "4", "5", "one", "two", "three", "four", "five"];

/// One pattern made into a sentence; the argument words are wrapped in
/// \u{1} ... \u{2} until they're split into words.
fn expand(p: &str, rng: &mut Rng) -> String {
    let mut out = String::new();
    let mut chars = p.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '[' => {
                let mut body = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    body.push(d);
                }
                let opts: Vec<&str> = body.split('|').collect();
                out.push_str(opts[rng.below(opts.len())]);
            }
            '{' => {
                let mut name = String::new();
                for d in chars.by_ref() {
                    if d == '}' {
                        break;
                    }
                    name.push(d);
                }
                let v = match name.as_str() {
                    "APP" => APPS[rng.below(APPS.len())].to_string(),
                    "FILE" => format!("{}{}", FILES[rng.below(FILES.len())], FILE_TAILS[rng.below(FILE_TAILS.len())]),
                    "WS" => WORKSPACES[rng.below(WORKSPACES.len())].to_string(),
                    "WHO" => PEOPLE[rng.below(PEOPLE.len())].to_string(),
                    "TOPIC" => TOPICS[rng.below(TOPICS.len())].to_string(),
                    "NUM" => NUMBERS[rng.below(NUMBERS.len())].to_string(),
                    "OUT" => format!("{} {}", OUT_VERBS[rng.below(OUT_VERBS.len())], OUT_OBJECTS[rng.below(OUT_OBJECTS.len())]),
                    _ => LAYERS[rng.below(LAYERS.len())].to_string(),
                };
                if matches!(name.as_str(), "APP" | "FILE" | "WS" | "WHO" | "TOPIC") {
                    out.push('\u{1}');
                    out.push_str(&v);
                    out.push('\u{2}');
                } else {
                    out.push_str(&v);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

const KEYS: &[&str] = &["qwertyuiop", "asdfghjkl", "zxcvbnm"];

/// A typo as people make them: two letters swapped, one missing, one
/// doubled, or a neighbouring key.
fn typo(w: &str, rng: &mut Rng) -> String {
    let mut c: Vec<char> = w.chars().collect();
    if c.len() < 4 {
        return w.to_string();
    }
    let i = 1 + rng.below(c.len() - 2);
    match rng.below(4) {
        0 => c.swap(i, i + 1),
        1 => {
            c.remove(i);
        }
        2 => c.insert(i, c[i]),
        _ => {
            if let Some(row) = KEYS.iter().find(|r| r.contains(c[i])) {
                let r: Vec<char> = row.chars().collect();
                let p = r.iter().position(|x| *x == c[i]).unwrap_or(0);
                c[i] = r[if p + 1 < r.len() && (p == 0 || rng.below(2) == 0) { p + 1 } else { p - 1 }];
            }
        }
    }
    c.into_iter().collect()
}

/// A sentence for intent i: words and which of them are the argument.
pub fn sentence(i: usize, rng: &mut Rng, typos: bool) -> Example {
    let (_, pats) = INTENTS[i];
    let mut s = expand(pats[rng.below(pats.len())], rng);
    // Fillers go around commands, not around chat ("hey nexa hello" is fine too).
    s = format!("{}{}{}", PREFIXES[rng.below(PREFIXES.len())], s, SUFFIXES[rng.below(SUFFIXES.len())]);
    let mut words = vec![];
    let mut arg = vec![];
    let mut inside = false;
    // Walk the characters to keep the markers.
    let mut cur = String::new();
    let flush = |cur: &mut String, words: &mut Vec<String>, arg: &mut Vec<bool>, inside: bool| {
        for w in crate::features::words(cur) {
            words.push(w);
            arg.push(inside);
        }
        cur.clear();
    };
    for ch in s.chars() {
        match ch {
            '\u{1}' => {
                flush(&mut cur, &mut words, &mut arg, inside);
                inside = true;
            }
            '\u{2}' => {
                flush(&mut cur, &mut words, &mut arg, inside);
                inside = false;
            }
            _ => cur.push(ch),
        }
    }
    flush(&mut cur, &mut words, &mut arg, inside);
    if typos && !words.is_empty() && rng.below(100) < 22 {
        let k = rng.below(words.len());
        words[k] = typo(&words[k], rng);
        if rng.below(100) < 15 && words.len() > 2 {
            let k2 = rng.below(words.len());
            words[k2] = typo(&words[k2], rng);
        }
    }
    Example { text: words.join(" "), intent: i, arg }
}

/// The whole set: `per` sentences for each intent (fewer for small ones,
/// so no intent drowns the others).
pub fn generate(per: usize, seed: u64, typos: bool) -> Vec<Example> {
    let mut rng = Rng::new(seed);
    let mut out = Vec::new();
    for i in 0..INTENTS.len() {
        // Names and chat vary most: twice the sentences.
        let n = match INTENTS[i].0 {
            "unknown" => per * 5,
            "launch" | "find_file" | "destroy" | "mail_find" => per * 2,
            _ => per,
        };
        for _ in 0..n {
            out.push(sentence(i, &mut rng, typos));
        }
    }
    rng.shuffle(&mut out);
    out
}
