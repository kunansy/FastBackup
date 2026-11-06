patch:
	@bumpversion --commit --tag patch

build-status:
	@curl -L --silent \
		-H "Accept: application/vnd.github+json" \
		-H "X-GitHub-Api-Version: 2022-11-28" \
		https://api.github.com/repos/kunansy/FastBackup/actions/runs \
		| jq '[.workflow_runs | .[] | select(.name == "Build docker image")] | .[0] | .name,.display_title,.status,.conclusion'

CURRENT_TAG := $(shell git describe --tags --abbrev=0)
LAST_TAG := $(shell git describe --tags --abbrev=0 HEAD^)

deploy:
	@echo "${LAST_TAG} -> ${CURRENT_TAG}"
	@ssh tracker "cd tracker; sed -i -E 's#(image: kunansy/fast-backup:)[0-9.]+#\1${CURRENT_TAG}#' docker-compose.yml; docker compose up -d --force-recreate tracker-backup; sleep 2; docker ps --filter name=tracker-backup --format json | jq '.Image,.State,.Status'"

.PHONY: patch build-status deploy
