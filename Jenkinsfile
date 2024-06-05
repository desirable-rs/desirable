pipeline {
    agent any

    stages {
        stage('Start') {
            steps {
                slackSend color: "good", message: " ${JOB_NAME} Branch: ${BRANCH_NAME} Build Start"
            }
        }
        stage('Fetch Code') {
            steps {
                checkout scmGit(branches: [[name: '**']], extensions: [], userRemoteConfigs: [[url: 'git@github.com:desirable-rs/desirable.git']])
            }
        }
        stage('Cargo Clippy') {
            steps {
                sh 'source /root/.cargo/env && cargo clippy'
            }
        }
        stage('Cargo Test') {
            steps {
                sh 'source /root/.cargo/env && cargo build --release'
            }
        }
        stage('Cargo Build') {
            steps {
                sh 'source /root/.cargo/env && cargo build --release'
            }
        }
        stage('Develop') {
            when {
                branch 'develop'
            }
            steps {
                echo 'main'  
            }
        }
        stage('Main') {
            when {
                branch 'main'
            }
            steps {
                echo 'main'
            }
        }
        stage('Notification') {
            steps {
                slackSend color: "good", message: "${JOB_NAME} Branch: ${BRANCH_NAME} Build Success"
            }
        }
    }
}
