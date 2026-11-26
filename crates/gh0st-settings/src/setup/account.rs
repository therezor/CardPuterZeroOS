//! The account: a name and a password, then the password again. Nothing is
//! made here; Done makes the user with what this step kept.

use super::Shared;
use gh0st_app::prelude::*;
use gh0st_core::sysconf;

const PASSWORD_MIN: usize = 4;

pub struct Account {
    label: String,
    a: Shared,
    form: Form,
    /// Set by a run cut short after the user was made: that name stays.
    made: Option<String>,
    /// The second screen, the password again.
    again: Option<Form>,
}

impl Account {
    pub fn new(label: &str, a: Shared) -> Account {
        let passwd = std::fs::read_to_string(super::path("/etc/passwd")).unwrap_or_default();
        let made = user_1000(&passwd);
        let form = match &made {
            Some(name) => Form::new().password(&format!("Password for {name}, 4 or more")),
            None => {
                let name = a.borrow().name.clone();
                Form::new()
                    .text("Your name on this device")
                    .with(&name)
                    .password("Password, 4 or more")
            }
        };
        Account {
            label: label.into(),
            a,
            form,
            made,
            again: None,
        }
    }

    fn password_at(&self) -> usize {
        usize::from(self.made.is_none())
    }

    /// The first screen's checks, then the second screen.
    fn check(&mut self) {
        if self.made.is_none() {
            let name = self.form.value(0).to_string();
            let etc = |f| std::fs::read_to_string(super::path(f)).unwrap_or_default();
            if !sysconf::valid_user_name(&name) {
                let why = "A letter first, then a to z, digits, - and _";
                return self.form.error(0, why);
            }
            if sysconf::name_taken(&etc("/etc/passwd"), &etc("/etc/group"), &name) {
                return self.form.error(0, "That name is taken");
            }
        }
        let p = self.password_at();
        if self.form.value(p).chars().count() < PASSWORD_MIN {
            return self.form.error(p, "Too short: 4 or more");
        }
        self.again = Some(Form::new().password("The password again"));
    }

    /// The second screen: the same password, so it is kept.
    fn confirm(&mut self, cx: &mut Cx) {
        let p = self.password_at();
        let Some(again) = self.again.as_mut() else {
            return;
        };
        if again.value(0) != self.form.value(p) {
            again.set(0, "");
            again.error(0, "They differ. Type it again.");
            return;
        }
        again.take(0).into_bytes().fill(0);
        let name = match &self.made {
            Some(n) => n.clone(),
            None => self.form.value(0).to_string(),
        };
        let mut a = self.a.borrow_mut();
        a.name = name;
        std::mem::take(&mut a.password).into_bytes().fill(0);
        a.password = self.form.take(p);
        drop(a);
        cx.done();
    }

    fn shown(&self) -> &Form {
        self.again.as_ref().unwrap_or(&self.form)
    }
}

/// The name UID 1000 has, in `/etc/passwd` text.
fn user_1000(passwd: &str) -> Option<String> {
    passwd.lines().find_map(|l| {
        let mut f = l.split(':');
        let name = f.next()?;
        (f.nth(1)? == "1000").then(|| name.to_string())
    })
}

impl Page for Account {
    fn title(&self) -> &str {
        "Account"
    }

    fn icon(&self) -> &str {
        "user"
    }

    fn context(&self) -> Option<&str> {
        Some(&self.label)
    }

    fn typing(&self) -> bool {
        true
    }

    fn bars(&self) -> Bars {
        Bars::top().hints(self.shown().hints("back", "next"))
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match self.again.as_mut() {
            Some(again) => {
                if again.key(key) == Some(Edit::Submit) {
                    self.confirm(cx);
                }
            }
            None => {
                if self.form.key(key) == Some(Edit::Submit) {
                    self.check();
                }
            }
        }
    }

    /// From the second screen, back to the first.
    fn back(&mut self, _: &mut Cx) -> bool {
        self.again.take().is_none()
    }

    fn draw(&self, ui: &mut Ui) {
        match (&self.again, &self.made) {
            (Some(again), _) => {
                ui.form(again);
                ui.note("You will need it for the lock screen, SSH and changes in Settings.");
            }
            (None, Some(name)) => {
                ui.note(&format!(
                    "{name} was made before the power went. Its password:"
                ));
                ui.form(&self.form);
            }
            (None, None) => {
                ui.form(&self.form);
                ui.note("The name is for SSH and the terminal.");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uid_1000_is_found_by_its_third_field() {
        let p = "root:x:0:0::/root:/bin/sh\nx1000:x:999:999::/:/bin/sh\nalex:x:1000:1000::/home/alex:/bin/bash\n";
        assert_eq!(user_1000(p).as_deref(), Some("alex"));
        assert_eq!(user_1000("root:x:0:0::/root:/bin/sh\n"), None);
    }
}
