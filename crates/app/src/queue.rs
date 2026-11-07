//! File d'attente : ordre de lecture, aléatoire, répétition.

use ondes_library::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    /// Reprend au début à la fin de la file.
    All,
    /// Rejoue le morceau en cours.
    One,
}

impl Repeat {
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

#[derive(Debug, Default)]
pub struct Queue {
    items: Vec<Track>,
    /// Ordre de lecture : indices dans `items` (mélangés en mode aléatoire).
    order: Vec<usize>,
    /// Position dans `order`.
    pos: usize,
    pub shuffle: bool,
    pub repeat: Repeat,
}

impl Queue {
    /// Remplace la file ; la lecture commence à `start` (indice dans `items`).
    pub fn set(&mut self, items: Vec<Track>, start: usize) {
        self.items = items;
        self.order = (0..self.items.len()).collect();
        self.pos = start.min(self.items.len().saturating_sub(1));
        if self.shuffle {
            self.reshuffle_keeping(start);
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.order.clear();
        self.pos = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn current(&self) -> Option<&Track> {
        self.items.get(*self.order.get(self.pos)?)
    }

    pub fn has_next(&self) -> bool {
        !self.is_empty() && (self.pos + 1 < self.order.len() || self.repeat != Repeat::Off)
    }

    /// Passe au morceau suivant. `auto` : fin naturelle du morceau
    /// (« répéter un » rejoue alors le même). `None` en fin de file.
    pub fn next(&mut self, auto: bool) -> Option<&Track> {
        if self.is_empty() {
            return None;
        }
        if auto && self.repeat == Repeat::One {
            return self.current();
        }
        if self.pos + 1 < self.order.len() {
            self.pos += 1;
        } else if self.repeat != Repeat::Off {
            if self.shuffle {
                fastrand::shuffle(&mut self.order);
            }
            self.pos = 0;
        } else {
            return None;
        }
        self.current()
    }

    pub fn prev(&mut self) -> Option<&Track> {
        if self.is_empty() {
            return None;
        }
        if self.pos > 0 {
            self.pos -= 1;
        } else if self.repeat == Repeat::All {
            self.pos = self.order.len() - 1;
        }
        self.current()
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        let Some(&current) = self.order.get(self.pos) else {
            return;
        };
        if on {
            self.reshuffle_keeping(current);
        } else {
            self.order = (0..self.items.len()).collect();
            self.pos = current;
        }
    }

    /// Mélange tout, avec `first` en tête (le morceau en cours ne change pas).
    fn reshuffle_keeping(&mut self, first: usize) {
        let mut rest: Vec<usize> = (0..self.items.len()).filter(|&i| i != first).collect();
        fastrand::shuffle(&mut rest);
        self.order = std::iter::once(first).chain(rest).collect();
        self.pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(n: usize) -> Vec<Track> {
        (0..n)
            .map(|i| Track {
                title: format!("t{i}"),
                ..Default::default()
            })
            .collect()
    }

    fn title(t: Option<&Track>) -> Option<String> {
        t.map(|t| t.title.clone())
    }

    #[test]
    fn plays_in_order_and_stops() {
        let mut q = Queue::default();
        q.set(items(3), 1);
        assert_eq!(title(q.current()), Some("t1".into()));
        assert_eq!(title(q.next(true)), Some("t2".into()));
        assert!(!q.has_next());
        assert_eq!(q.next(true), None);
        assert_eq!(title(q.prev()), Some("t1".into()));
        assert_eq!(title(q.prev()), Some("t0".into()));
        assert_eq!(title(q.prev()), Some("t0".into())); // début : reste sur le premier
    }

    #[test]
    fn repeat_modes() {
        let mut q = Queue::default();
        q.set(items(2), 1);
        q.repeat = Repeat::One;
        assert_eq!(title(q.next(true)), Some("t1".into())); // fin naturelle : même morceau
        assert_eq!(title(q.next(false)), Some("t0".into())); // « suivant » : on avance
        q.repeat = Repeat::All;
        assert_eq!(title(q.next(true)), Some("t1".into()));
        assert_eq!(title(q.next(true)), Some("t0".into())); // boucle
        assert_eq!(title(q.prev()), Some("t1".into()));
        assert_eq!(Repeat::Off.cycle().cycle().cycle(), Repeat::Off);
    }

    #[test]
    fn shuffle_keeps_current_and_plays_everything_once() {
        let mut q = Queue::default();
        q.set(items(50), 7);
        q.set_shuffle(true);
        assert_eq!(title(q.current()), Some("t7".into()));
        let mut seen = vec![q.current().unwrap().title.clone()];
        while let Some(t) = q.next(true) {
            seen.push(t.title.clone());
        }
        seen.sort();
        let mut all: Vec<_> = items(50).into_iter().map(|t| t.title).collect();
        all.sort();
        assert_eq!(seen, all);

        // Retour à l'ordre normal depuis le morceau en cours.
        q.set_shuffle(false);
        let current = q.current().unwrap().title.clone();
        let n: usize = current[1..].parse().unwrap();
        assert_eq!(
            title(q.next(false)),
            (n + 1 < 50).then(|| format!("t{}", n + 1))
        );
    }
}
