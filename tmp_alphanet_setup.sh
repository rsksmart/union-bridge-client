git fetch
git checkout chore/alphanet_fixes
sed -i 's/export UC_TAG=.*/export UC_TAG="latest-alphanet"/' .envrc
direnv allow
bash cli-setup-operators.sh --env alphanet --op "${UC_OPERATOR_ID}"
cd docker/operator
bash start_operators.sh --env alphanet pull --policy always