import sys
from PyQt6.QtWidgets import QApplication, QMainWindow, QLabel, QVBoxLayout, QWidget, QPushButton
import requests

class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("CatRemote")
        self.resize(400, 300)

        layout = QVBoxLayout()

        self.status_label = QLabel("Status: Disconnected")
        layout.addWidget(self.status_label)

        self.btn_fetch = QPushButton("Ping API")
        self.btn_fetch.clicked.connect(self.ping_api)
        layout.addWidget(self.btn_fetch)

        container = QWidget()
        container.setLayout(layout)
        self.setCentralWidget(container)

    def ping_api(self):
        try:
            # Assume FastAPI backend runs on 8000
            response = requests.get("http://127.0.0.1:8000/api/config")
            if response.status_code == 200:
                data = response.json()
                self.status_label.setText(f"API Ping Success! Preferred: {data.get('preferred_protocol')}")
            else:
                self.status_label.setText("API Error: Bad Status")
        except Exception as e:
            self.status_label.setText(f"API Error: {str(e)}")

if __name__ == "__main__":
    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())
