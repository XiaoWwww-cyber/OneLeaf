import sqlite3
import os

app_data = os.path.expandvars(r"%APPDATA%\com.oneleaf.dev")
db_path = os.path.join(app_data, "knowledge_base.db")

print(f"Checking DB: {db_path}")
if not os.path.exists(db_path):
    print("DB does not exist.")
else:
    conn = sqlite3.connect(db_path)
    c = conn.cursor()
    c.execute("SELECT document_id, dimension, length(embedding) FROM document_vectors LIMIT 5")
    rows = c.fetchall()
    for row in rows: # (doc_id, dimension, length_in_bytes)
        print(row)
        
    c.execute("SELECT dimension, count(*) FROM document_vectors GROUP BY dimension")
    print("Dimension stats:", c.fetchall())
