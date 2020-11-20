

#[derive(Debug)]
enum CharLevel {
    XLvl(u32),
    XLvlwExp(u32, u32),
    HD(u32)
}

#[derive(Debug)]
enum Strength {
    Normal(u32),
    Percentile(u32, u32)
}

#[derive(Debug)]
struct AbilityScores {
    strength: Strength,
    dexterity: u32,
    constitution: u32,
    intelligence: u32,
    wisdom: u32,
    charisma: u32
}

#[derive(Debug)]
enum Class {
    Rank(String),
    Polyform(String)
}

#[derive(Debug)]
enum Align {
    Lawful,
    Neutral,
    Chaotic,
    Unaligned
}

#[derive(Debug)]
struct NHStats {
    dlvl: u32,
    gold: u32,
    hp: u32,
    maxhp: u32,
    pw: u32,
    maxpw: u32,
    armour_class: i32,
    level: CharLevel,
    turns: Option<u32>,
    score: Option<u32>,
    ability: AbilityScores,
    align: Align,
    name: String,
    rank: Class
}

impl NHStats {
    fn new() -> Self {
        NHStats {
            dlvl: 1,
            gold: 0,
            hp: 1,
            maxhp: 10,
            pw: 5,
            maxpw: 10,
            armour_class: 10,
            level: CharLevel::XLvl(1),
            turns: None,
            score: None,
            ability: AbilityScores {
                strength: Strength::Normal(10),
                dexterity: 10,
                constitution: 10,
                intelligence: 10,
                wisdom: 10,
                charisma: 10
            },
            align: Align::Unaligned,
            name: String::from("luser"),
            rank: Class::Rank(String::from("windows hacker"))
        }
    }

    fn read_statusline(&mut self, window: &SubWindow) -> Result<()> {
        let mut saved_tokens: Vec<String> = Vec::new();
        for line in window.get_lines()? {
            for token in line.split_whitespace().clone() {
                let split_vec: Vec<&str> = token.splitn(2, ':').collect();
                if split_vec.len() == 1 {
                    saved_tokens.push(split_vec[0].to_string());
                } else {
                    let (field, value) = (split_vec[0], split_vec[1]);
                    match field {
                        "Dlvl" => if let Ok(n) = value.parse::<u32>() {
                            self.dlvl = n;
                        },
                        "$" => if let Ok(n) = value.parse::<u32>() {
                            self.gold = n;
                        },
                        "HP" => {
                            let split_maxhp: Vec<&str> = value.split(|c| c == '(' || c == ')').collect();
                            if split_maxhp.len() >= 2 {
                                if let Ok(n) = split_maxhp[0].parse::<u32>() {
                                    self.hp = n;
                                }
                                if let Ok(n) = split_maxhp[1].parse::<u32>() {
                                    self.maxhp = n;
                                }
                            }
                        },
                        "Pw" => {
                            let split_maxpw: Vec<&str> = value.split(|c| c == '(' || c == ')').collect();
                            if split_maxpw.len() >= 2 {
                                if let Ok(n) = split_maxpw[0].parse::<u32>() {
                                    self.pw = n;
                                }
                                if let Ok(n) = split_maxpw[1].parse::<u32>() {
                                    self.maxpw = n;
                                }
                            }
                        },
                        "AC" => if let Ok(n) = value.parse::<i32>() {
                            self.armour_class = n;
                        },
                        "HD" => if let Ok(n) = value.parse::<u32>() {
                            self.level = CharLevel::HD(n);
                        },
                        "Xp" => {
                            let split_xp: Vec<&str> = value.splitn(2, '/').collect();
                            if split_xp.len() == 1 {
                                if let Ok(n) = split_xp[0].parse::<u32>() {
                                    self.level = CharLevel::XLvl(n);
                                }
                            } else if split_xp.len() == 2 {
                                if let Ok(nxp) = split_xp[0].parse::<u32>() {
                                    if let Ok(xp_points) = split_xp[1].parse::<u32>() {
                                        self.level = CharLevel::XLvlwExp(nxp, xp_points);
                                    }
                                }
                            }
                        },
                        "T" => if let Ok(n) = value.parse::<u32>() {
                            self.turns = Some(n);
                        },
                        "S" => if let Ok(n) = value.parse::<u32>() {
                            self.score = Some(n);
                        },
                        "St" => {
                            let st_split: Vec<&str> = value.splitn(2,'/').collect();
                            if st_split.len() == 1 {
                                if let Ok(n) = st_split[0].parse::<u32>() {
                                    self.ability.strength = Strength::Normal(n);
                                }
                            } else if st_split.len() == 2 {
                                if let Ok(n) = st_split[0].parse::<u32>() {
                                    match st_split[1] {
                                        "**" => self.ability.strength = Strength::Percentile(n, 100),
                                        _ => if let Ok(perc) = st_split[1].parse::<u32>() {
                                            self.ability.strength = Strength::Percentile(n, perc);
                                        }
                                    }
                                }
                            }
                        },
                        "Dx" => if let Ok(n) = value.parse::<u32>() {
                            self.ability.dexterity = n;
                        },
                        "Co" => if let Ok(n) = value.parse::<u32>() {
                            self.ability.constitution = n;
                        },
                        "In" => if let Ok(n) = value.parse::<u32>() {
                            self.ability.intelligence = n;
                        },
                        "Wi" => if let Ok(n) = value.parse::<u32>() {
                            self.ability.wisdom = n;
                        },
                        "Ch" => if let Ok(n) = value.parse::<u32>() {
                            self.ability.charisma = n;
                        },
                        _ => saved_tokens.push(token.to_string())
                    }
                }
            }
        }

        // now with the number stuff out the way we try to do the
        // remaning considerations
        if let Some(last_token) = saved_tokens.pop() {
            match last_token.as_str() {
                "Lawful" => self.align = Align::Lawful,
                "Neutral" => self.align = Align::Neutral,
                "Chaotic" => self.align = Align::Chaotic,
                "Unaligned" => self.align = Align::Unaligned,
                _ => ()
            }
        }

        for i in 0 .. saved_tokens.len() {
            if i >= 1 && saved_tokens[i].as_str() == "the" && i < saved_tokens.len() {
                self.name = saved_tokens[i-1].clone();
                match self.level {
                    CharLevel::HD(_) => self.rank = Class::Polyform(saved_tokens[i+1].clone()),
                    _ =>                self.rank = Class::Rank(saved_tokens[i+1].clone())
                }
            }
        }

        Ok(())
    }
}

type NHInv = Vec<NHInvItem>;

enum ItemClass {
    Weapons,
    Armour,
    Comestibles,
    Wands,
    Rings,
    Potions,
    Scrolls,
    Spellbooks,
    Tools,
    GemsnStones
}

enum BUC {
    Blessed,
    Uncursed,
    Cursed
}

enum WearType {
    Corroded,
    Rusty,
    Burnt,
    Rotted
}

enum WearExtent {
    None,
    Some,
    Very,
    Thoroughly
}

struct Wear {
    e_type: WearType,
    e_extent: WearExtent
}

struct NHInvItem {
    item: ItemClass,
    inventory_letter: char, // strictly speaking A-Z
    beatitude: Option<BUC>,
    erosion: Wear,
    charges: Option<u32>,
    enchantment: Option<u32>,
    fooproofed: bool,
    greased: bool,
    description: String,
    name: String
}

struct NetHackData {
    windows: Vec<SubWindow>,
    //level_map: NHMap,
    inventory: NHInv,
    status: NHStats
}

impl NetHackData {
    pub fn new() -> Self {
        NetHackData {
            windows: Vec::new(),
            inventory: Vec::new(),
            status: NHStats::new()
        }
    }

    pub fn update(&mut self, term: &GameScreen) -> Result<()> {
        self.windows.clear();
        let sub_windows = term.get_subwindows()?;
        for window in sub_windows {
            self.windows.push(window);
        }

        for window in self.windows.clone() {
            let (height, width) = window.get_size();
            write!(stderr(), "win size: {} by {}\n", height, width);
            if height == 2 {
                // statusline!
                self.status.read_statusline(&window);
                write!(stderr(), "{:?}\n", self.status);
            }
        }

        Ok(())
    }

    pub fn debug(&self, stderr: &mut Stderr) {
        let mut window_nr = 1;

        for win in self.windows.clone() {
            write!(stderr, "this is the {}th window\n", window_nr);
            if let Ok(line_vec) = win.get_lines() {
                for line in line_vec {
                    write!(stderr, "{}\n", line);
                }
            }
            window_nr += 1;
        }
    }

    

    //try_inventory(&mut self, window: &SubWindow) -> Result<()> {

    //}
}