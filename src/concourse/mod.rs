use super::config::*;
use handlebars::*;
use serde::*;

const BASE_TEMPLATE: &str = include_str!("base.yml");
const BASE_TEMPLATE_NAME: &str = "base";
const RESOURCE_PARTIAL: &str = include_str!("resource.yml");
const RESOURCE_PARTIAL_NAME: &str = "resource";
const JOB_PARTIAL: &str = include_str!("job.yml");
const JOB_PARTIAL_NAME: &str = "job";
const USER_IMAGE_RESOURCE: &str = "user_image_resource";
const USER_RUN: &str = "user_run";

pub struct Concourse {
    handlebars: Handlebars<'static>,
    config: Config,
}

impl Concourse {
    pub fn new(config: Config) -> Self {
        let mut handlebars = Handlebars::new();
        handlebars
            .register_partial(RESOURCE_PARTIAL_NAME, RESOURCE_PARTIAL)
            .unwrap();
        handlebars
            .register_partial(JOB_PARTIAL_NAME, JOB_PARTIAL)
            .unwrap();
        handlebars
            .register_partial(
                USER_IMAGE_RESOURCE,
                &user_image_resource(&config.concourse.as_ref().unwrap().task.image_resource),
            )
            .unwrap();
        handlebars
            .register_partial(
                USER_RUN,
                &user_run(&config.concourse.as_ref().unwrap().task.run),
            )
            .unwrap();
        handlebars
            .register_template_string(BASE_TEMPLATE_NAME, BASE_TEMPLATE)
            .unwrap();
        Self { handlebars, config }
    }

    pub fn render_pipeline(&self) -> String {
        let repo = self.repo_conf();
        let data = ConcourseData {
            jobs: self.get_jobs(),
            resources: self.get_resources(),
            repo_uri: &repo.uri,
            branch: &repo.branch,
            github_private_key: &repo.private_key,
        };
        self.handlebars.render(BASE_TEMPLATE_NAME, &data).unwrap()
    }

    fn get_jobs(&self) -> Vec<JobData> {
        let mut jobs = Vec::new();
        for env in self.environments() {
            jobs.push(JobData {
                name: &env.name,
                has_head: !env.head_filters().is_empty(),
                passed: env.propagated_from(),
            })
        }
        jobs
    }

    fn get_resources(&self) -> Vec<Resource> {
        let repo = self.repo_conf();
        let mut resources = Vec::new();
        for env in self.environments() {
            if !env.head_filters().is_empty() {
                resources.push(Resource {
                    name: head_resource_name(env),
                    repo_uri: &repo.uri,
                    branch: &repo.branch,
                    paths: env.head_filters(),
                    github_private_key: &repo.private_key,
                });
            }
        }
        resources
    }

    fn environments(&self) -> impl Iterator<Item = &EnvironmentConfig> {
        self.config.environments.values()
    }
    fn concourse_conf(&self) -> &ConcourseConfig {
        &self.config.concourse.as_ref().unwrap()
    }
    fn repo_conf(&self) -> &RepoConfig {
        &self.concourse_conf().repo
    }
}

fn head_resource_name(env: &EnvironmentConfig) -> String {
    format!("{}-head", env.name)
}
fn user_image_resource(image: &serde_yaml::Value) -> String {
    let mut res = String::new();
    for line in serde_yaml::to_string(&image)
        .expect("Couldn't serialize image")
        .split("\n")
        .skip(1)
    {
        res.push_str("        ");
        res.push_str(line);
        res.push_str("\n")
    }
    res.trim_end_matches("\n").to_string()
}
fn user_run(run: &serde_yaml::Value) -> String {
    let mut res = String::new();
    for line in serde_yaml::to_string(&run)
        .expect("Couldn't serialize image")
        .split("\n")
        .skip(1)
    {
        res.push_str("        ");
        res.push_str(line);
        res.push_str("\n")
    }
    res.trim_end_matches("\n").to_string()
}

#[derive(Debug, Serialize)]
struct ConcourseData<'a> {
    jobs: Vec<JobData<'a>>,
    resources: Vec<Resource<'a>>,
    repo_uri: &'a str,
    branch: &'a str,
    github_private_key: &'a str,
}
#[derive(Debug, Serialize)]
struct JobData<'a> {
    name: &'a String,
    has_head: bool,
    passed: Option<&'a String>,
}
#[derive(Debug, Serialize)]
struct Resource<'a> {
    name: String,
    repo_uri: &'a str,
    branch: &'a str,
    github_private_key: &'a str,
    paths: &'a [String],
}
