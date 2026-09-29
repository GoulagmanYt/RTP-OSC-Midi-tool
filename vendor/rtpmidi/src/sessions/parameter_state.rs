//! Bounded receiver knowledge of RPN/NRPN transactions (RP015 reset semantics).
#[derive(Clone, Copy, Default)]
pub(super) struct ParameterValue {
    pub msb: Option<u8>,
    pub lsb: Option<u8>,
    pub buttons: Option<i16>,
    pub count: Option<u8>,
}

#[derive(Clone, Copy)]
pub(super) struct ParameterState {
    pub selected: Option<u16>,
    pub pending: Option<(bool, u8)>,
    msbs: [Option<u8>; 2],
    entries: [Option<(u16, ParameterValue)>; 64],
    replacement: usize,
}

impl Default for ParameterState {
    fn default() -> Self {
        Self {
            selected: None,
            pending: None,
            msbs: [None; 2],
            entries: [None; 64],
            replacement: 0,
        }
    }
}

impl ParameterState {
    pub fn get(&self, key: u16) -> ParameterValue {
        self.entries
            .iter()
            .flatten()
            .find(|(k, _)| *k == key)
            .map_or(ParameterValue::default(), |(_, value)| *value)
    }

    pub fn set(&mut self, key: u16, value: ParameterValue) {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.is_some_and(|(k, _)| k == key))
            .or_else(|| self.entries.iter().position(Option::is_none))
            .unwrap_or_else(|| {
                let index = self.replacement;
                self.replacement = (index + 1) % self.entries.len();
                index
            });
        self.entries[index] = Some((key, value));
    }

    fn initiate(&mut self, nrpn: bool, msb: u8, lsb: u8) {
        self.pending = None;
        let number = u16::from(msb) * 128 + u16::from(lsb);
        self.selected = (number != 16383).then_some(number | if nrpn { 16384 } else { 0 });
        if let Some(key) = self.selected {
            let mut value = self.get(key);
            value.count = value.count.map(|count| count.wrapping_add(1) & 127);
            self.set(key, value);
        }
    }

    pub fn observe(&mut self, number: u8, value: u8) {
        match number {
            99 | 101 => {
                let nrpn = number == 99;
                self.msbs[usize::from(nrpn)] = Some(value);
                self.pending = Some((nrpn, value));
                self.selected = None;
            }
            98 | 100 => {
                let nrpn = number == 98;
                if let Some(msb) = self.msbs[usize::from(nrpn)] {
                    self.initiate(nrpn, msb, value);
                } else {
                    self.pending = None;
                    self.selected = None;
                }
            }
            6 | 38 | 96 | 97 => {
                if let Some((nrpn, msb)) = self.pending {
                    self.initiate(nrpn, msb, 0);
                }
                if let Some(key) = self.selected {
                    let mut entry = self.get(key);
                    match number {
                        6 => {
                            entry.msb = Some(value);
                            entry.lsb = Some(0);
                            entry.buttons = Some(0);
                        }
                        38 => {
                            entry.lsb = Some(value);
                            entry.buttons = Some(0);
                        }
                        _ => {
                            entry.buttons = entry.buttons.map(|count| {
                                (count + if number == 96 { 1 } else { -1 }).clamp(-16383, 16383)
                            });
                        }
                    }
                    self.set(key, entry);
                }
            }
            121 => {
                self.selected = None;
                self.pending = None;
                self.msbs = [None; 2];
            }
            _ => {}
        }
    }
}
