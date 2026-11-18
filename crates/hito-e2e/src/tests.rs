use super::*;

const P: ProjectId = ProjectId(7);

/// The part of the server that matters here: one log, one epoch, one GroupInfo.
#[derive(Default)]
struct Server {
    log: Vec<Entry>,
    epoch: u64,
    info: Option<Vec<u8>>,
}

impl Server {
    fn append(&mut self, author: UserId, data: Vec<u8>) -> i64 {
        let seq = self.log.len() as i64 + 1;
        self.log.push(Entry { seq, author, data });
        seq
    }

    fn commit(&mut self, author: UserId, p: &Pending) -> std::result::Result<i64, ()> {
        let h = inspect(&p.commit).unwrap();
        if h.epoch != self.epoch || h.kind != Kind::Commit || h.group_id != group_id(P) {
            return Err(());
        }
        let (gid, epoch) = inspect_group_info(&p.group_info).unwrap();
        assert_eq!(gid, group_id(P));
        assert_eq!(epoch, self.epoch + 1);
        self.epoch = epoch;
        self.info = Some(p.group_info.clone());
        Ok(self.append(author, p.commit.clone()))
    }

    fn post(&mut self, d: &mut Device, (data, epoch): (Vec<u8>, u64)) {
        let h = inspect(&data).unwrap();
        assert_eq!((h.epoch, h.kind), (self.epoch, Kind::Application));
        assert_eq!(epoch, self.epoch);
        let seq = self.append(d.user(), data);
        d.sent(P, seq);
    }

    fn sync(&self, d: &mut Device) -> Vec<Outcome> {
        let from = d.last_seq(P);
        self.log
            .iter()
            .filter(|e| e.seq > from)
            .map(|e| d.process(P, e).unwrap())
            .filter(|o| *o != Outcome::Skipped)
            .collect()
    }

    /// Enter the project's group: create it with a first key, or join it from the GroupInfo.
    fn enter(&mut self, d: &mut Device) {
        let from = self.log.len() as i64;
        match self.info.clone() {
            None => {
                self.info = Some(d.create(P, from).unwrap());
                d.confirm(P, None).unwrap();
                let share = d.rotate(P).unwrap();
                self.post(d, share);
            }
            Some(info) => {
                let p = d.join(P, &info, from).unwrap();
                let seq = self.commit(d.user(), &p).unwrap();
                d.confirm(P, Some(seq)).unwrap();
            }
        }
    }
}

fn device(user: i64) -> Device {
    Device::new(UserId(user)).unwrap()
}

#[test]
fn a_late_member_gets_every_key_and_reads_what_came_before() {
    let mut server = Server::default();
    let (mut ana, mut ben) = (device(1), device(2));
    server.enter(&mut ana);
    let early = ana.seal(P, Field::Task, "Revisar el presupuesto 🙂").unwrap();

    server.enter(&mut ben);
    assert!(!ben.has_keys(P));
    let out = server.sync(&mut ana);
    assert!(matches!(out.as_slice(), [Outcome::Changed { joined, .. }] if joined[0].user == UserId(2)));
    let share = ana.share_keys(P).unwrap();
    server.post(&mut ana, share);
    assert_eq!(server.sync(&mut ben), vec![Outcome::Keys { new: 1 }]);
    assert_eq!(
        ben.open(P, Field::Task, &early).unwrap(),
        "Revisar el presupuesto 🙂"
    );

    for e in &server.log {
        assert!(!String::from_utf8_lossy(&e.data).contains("presupuesto"));
    }
    assert!(!String::from_utf8_lossy(&early.data).contains("presupuesto"));
}

#[test]
fn sealed_text_is_bound_to_its_project_and_field() {
    let mut server = Server::default();
    let mut ana = device(1);
    server.enter(&mut ana);
    let sealed = ana.seal(P, Field::Comment, "secreto").unwrap();
    assert!(matches!(ana.open(P, Field::Task, &sealed), Err(Error::Tampered)));
    assert!(matches!(
        ana.open(ProjectId(8), Field::Comment, &sealed),
        Err(Error::NoKey)
    ));
    let mut flipped = sealed.clone();
    if let Some(b) = flipped.data.last_mut() {
        *b ^= 1;
    }
    assert!(matches!(ana.open(P, Field::Comment, &flipped), Err(Error::Tampered)));
    let short = Sealed {
        key_id: sealed.key_id,
        data: vec![0; 5],
    };
    assert!(matches!(ana.open(P, Field::Comment, &short), Err(Error::Tampered)));
    // Two seals of one text differ: every one has its own nonce.
    assert_ne!(ana.seal(P, Field::Comment, "secreto").unwrap(), sealed);
}

#[test]
fn removal_rotates_the_key_and_the_removed_device_gets_no_new_one() {
    let mut server = Server::default();
    let (mut ana, mut ben) = (device(1), device(2));
    server.enter(&mut ana);
    server.enter(&mut ben);
    server.sync(&mut ana);
    let share = ana.share_keys(P).unwrap();
    server.post(&mut ana, share);
    server.sync(&mut ben);
    let old = ana.current_key(P).unwrap();

    let p = ana
        .remove(P, |leaf| leaf.user != UserId(2))
        .unwrap()
        .unwrap();
    let seq = server.commit(UserId(1), &p).unwrap();
    ana.confirm(P, Some(seq)).unwrap();
    let share = ana.rotate(P).unwrap();
    server.post(&mut ana, share);
    let secret = ana.seal(P, Field::Task, "solo para Ana").unwrap();
    assert!(secret.key_id > old);

    assert!(server.sync(&mut ben).contains(&Outcome::Removed));
    assert!(!ben.is_member(P));
    assert!(matches!(ben.open(P, Field::Task, &secret), Err(Error::NoKey)));
}

#[test]
fn a_saved_device_keeps_its_keys() {
    let mut server = Server::default();
    let mut ana = device(1);
    server.enter(&mut ana);
    let sealed = ana.seal(P, Field::Milestone, "Lanzamiento").unwrap();
    let ana = Device::from_bytes(&ana.to_bytes().unwrap()).unwrap();
    assert_eq!(ana.open(P, Field::Milestone, &sealed).unwrap(), "Lanzamiento");
    assert!(ana.is_member(P));
}

#[test]
fn a_known_key_cannot_be_replaced_and_a_forged_author_is_refused() {
    let mut server = Server::default();
    let (mut ana, mut ben, mut cleo) = (device(1), device(2), device(3));
    server.enter(&mut ana);
    server.enter(&mut ben);
    server.enter(&mut cleo);
    server.sync(&mut ana);
    server.sync(&mut ben);
    let share = ana.share_keys(P).unwrap();
    server.post(&mut ana, share);
    server.sync(&mut ben);
    server.sync(&mut cleo);
    let sealed = ana.seal(P, Field::Task, "real").unwrap();

    // Cleo forgets her keys and starts her own key 1: the others keep Ana's.
    cleo.forget_keys(P);
    let share = cleo.rotate(P).unwrap();
    server.post(&mut cleo, share);
    assert_eq!(server.sync(&mut ben), vec![Outcome::Keys { new: 0 }]);
    assert_eq!(ben.open(P, Field::Task, &sealed).unwrap(), "real");

    // A server that says Ben wrote Ana's share gets an unreadable entry.
    let (data, _) = ana.share_keys(P).unwrap();
    server.append(UserId(2), data);
    assert_eq!(server.sync(&mut cleo).last(), Some(&Outcome::Unreadable));
}

#[test]
fn an_offline_device_is_added_with_its_key_package() {
    let mut server = Server::default();
    let (mut ana, mut ben) = (device(1), device(2));
    server.enter(&mut ana);
    let before = ana.seal(P, Field::Task, "antes").unwrap();
    let kp = ben.key_packages(1).unwrap();
    let leaf = Leaf {
        user: UserId(2),
        public_key: ben.public_key(),
    };
    let (p, welcome) = ana.add(P, &[(leaf, kp[0].clone())]).unwrap().unwrap();
    let seq = server.commit(UserId(1), &p).unwrap();
    ana.confirm(P, Some(seq)).unwrap();
    let share = ana.share_keys(P).unwrap();
    server.post(&mut ana, share);

    ben.welcome(P, &welcome, seq).unwrap();
    server.sync(&mut ben);
    assert_eq!(ben.open(P, Field::Task, &before).unwrap(), "antes");
}

#[test]
fn garbage_is_refused_without_panicking() {
    assert!(inspect(b"").is_err());
    assert!(inspect(&[0xff; 64]).is_err());
    assert!(inspect_group_info(b"nope").is_err());
    assert!(Device::from_bytes(b"{}").is_err());
    let mut d = device(1);
    assert!(d.join(P, b"junk", 0).is_err());
    assert!(matches!(d.seal(P, Field::Task, "x"), Err(Error::NoKey)));
    assert!(matches!(d.share_keys(P), Err(Error::NoGroup)));
    let out = d
        .process(
            P,
            &Entry {
                seq: 1,
                author: UserId(2),
                data: vec![1, 2, 3],
            },
        )
        .unwrap();
    assert_eq!(out, Outcome::Unreadable);
    assert_eq!(fingerprint(b"k").split(' ').count(), 6);
}
