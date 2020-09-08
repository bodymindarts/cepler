use crate::config::*;
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

pub struct ConcourseGen {
    handlebars: Handlebars<'static>,
    config: Config,
    path_to_config: String,
}

impl ConcourseGen {
    pub fn new(config: Config, path_to_config: String) -> Self {
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
        Self {
            handlebars,
            config,
            path_to_config,
        }
    }

    pub fn render_pipeline(&self) -> String {
        let resources = self.get_resources();
        let data = ConcourseGenData {
            jobs: self.get_jobs(),
            resources,
        };
        self.handlebars.render(BASE_TEMPLATE_NAME, &data).unwrap()
    }

    fn get_jobs(&self) -> Vec<JobData> {
        let repo = self.repo_conf();
        let mut jobs = Vec::new();
        for env in self.environments() {
            jobs.push(JobData {
                name: &env.name,
                has_head: !env.head_filters().is_empty(),
                passed: env.propagated_from(),
                repo_uri: &repo.uri,
                branch: &repo.branch,
                git_private_key: &repo.private_key,
            })
        }
        jobs
    }

    fn get_resources(&self) -> Vec<Resource> {
        let repo = self.repo_conf();
        let mut resources = Vec::new();
        for env in self.environments() {
            resources.push(Resource {
                name: env.name.clone(),
                r#type: "cepler",
                repo_uri: &repo.uri,
                branch: &repo.branch,
                git_private_key: &repo.private_key,
                path_to_config: &self.path_to_config,
            });
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
struct ConcourseGenData<'a> {
    jobs: Vec<JobData<'a>>,
    resources: Vec<Resource<'a>>,
}
#[derive(Debug, Serialize)]
struct JobData<'a> {
    name: &'a String,
    has_head: bool,
    passed: Option<&'a String>,
    repo_uri: &'a str,
    branch: &'a str,
    git_private_key: &'a str,
}
#[derive(Debug, Serialize)]
struct Resource<'a> {
    name: String,
    r#type: &'static str,
    repo_uri: &'a str,
    branch: &'a str,
    git_private_key: &'a str,
    path_to_config: &'a str,
}
