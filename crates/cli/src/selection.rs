//! Pin 选择规则：精确名称与正则取并集，并验证配置中的名称集合。

use regex::Regex;

#[derive(Debug)]
pub struct Selection {
    names: Vec<String>,
    filter: Option<Regex>,
}

impl Selection {
    pub fn new(names: Vec<String>, filter: Option<Regex>) -> Self {
        Self { names, filter }
    }

    pub fn matches(&self, name: &str) -> bool {
        // 精确名称与正则取并集；两者都未提供时选择全部 Pin。
        (self.names.is_empty() && self.filter.is_none())
            || self.names.iter().any(|selected| selected == name)
            || self.filter.as_ref().is_some_and(|filter| filter.is_match(name))
    }

    pub fn validate<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
        // 显式名称必须全部存在，不能因正则命中了其他项而忽略拼写错误。
        let names: Vec<_> = names.into_iter().collect();
        let missing: Vec<_> = self
            .names
            .iter()
            .filter(|selected| !names.contains(&selected.as_str()))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(format!("unknown pin name(s): {}", missing.join(", ")));
        }
        if self.names.is_empty() && self.filter.is_some() && !names.iter().any(|name| self.matches(name)) {
            return Err("filter matched no pins".into());
        }
        Ok(())
    }

    pub fn is_all(&self) -> bool {
        self.names.is_empty() && self.filter.is_none()
    }
}
