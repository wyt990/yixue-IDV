import { Navigate, Route, Routes } from "react-router-dom";
import { useAuth } from "./auth";
import { AppLayout } from "./layout";
import { AdminsPage } from "./pages/AdminsPage";
import { AppearancePage } from "./pages/AppearancePage";
import { ClientsPage } from "./pages/ClientsPage";
import { DashboardPage } from "./pages/DashboardPage";
import { EdgesPage } from "./pages/EdgesPage";
import { ImagesPage } from "./pages/ImagesPage";
import { LabsPage } from "./pages/LabsPage";
import { LoginPage } from "./pages/LoginPage";
import { PeSessionsPage } from "./pages/PeSessionsPage";
import { ModesPage } from "./pages/ModesPage";
import { Phase3Page } from "./pages/Phase3Page";
import { TasksPage } from "./pages/TasksPage";

function Protected({ children }: { children: React.ReactNode }) {
  const { user, loading } = useAuth();
  if (loading) return <div className="login-page muted">加载中…</div>;
  if (!user) return <Navigate to="/login" replace />;
  return children;
}

export default function App() {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route
        path="/"
        element={
          <Protected>
            <AppLayout />
          </Protected>
        }
      >
        <Route index element={<DashboardPage />} />
        <Route path="labs" element={<LabsPage />} />
        <Route path="clients" element={<ClientsPage />} />
        <Route path="images" element={<ImagesPage />} />
        <Route path="edges" element={<EdgesPage />} />
        <Route path="pe-sessions" element={<PeSessionsPage />} />
        <Route path="tasks" element={<TasksPage />} />
        <Route path="modes" element={<ModesPage />} />
        <Route path="phase3" element={<Phase3Page />} />
        <Route path="admins" element={<AdminsPage />} />
        <Route path="appearance" element={<AppearancePage />} />
      </Route>
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}
